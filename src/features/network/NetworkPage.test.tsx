import { act, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { countChips, stateBadges } from '../../shared/testing';
import type { NetworkRowView, NetworkSnapshot, RowEndpointView, Section } from '../../shared/ipc';
import { NetworkPage } from './NetworkPage';

const { subscribeToNetwork, fetchCoreStatus, subscribeToHeartbeat } = vi.hoisted(() => ({
  subscribeToNetwork: vi.fn(),
  fetchCoreStatus: vi.fn(),
  subscribeToHeartbeat: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({
  subscribeToNetwork,
  fetchCoreStatus,
  subscribeToHeartbeat,
}));

const endpoint = (overrides: Partial<RowEndpointView> = {}): RowEndpointView => ({
  key: 'foreign/steam/api',
  writtenAddress: 'api.steampowered.com',
  resolvedAddress: '203.0.113.7:443',
  tunnelled: false,
  measurable: true,
  probeKind: 'tcpConnect',
  filteringConfirmed: false,
  health: 'ok',
  rttMs: 42,
  meanRttMs: 45,
  jitterMs: 2,
  lossPct: 0,
  checks: [{ ageSecs: -45, mark: 'answered' }],
  ...overrides,
});

const row = (overrides: Partial<NetworkRowView> = {}): NetworkRowView => ({
  key: 'foreign/steam',
  label: 'Steam',
  health: 'ok',
  counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  rttMs: 42,
  lastCheckedSecs: 12,
  endpoints: [endpoint()],
  ...overrides,
});

const section = (which: Section, rows: NetworkRowView[]) => ({
  section: which,
  readByVerdict: which === 'domestic' || which === 'foreign',
  verdict: 'ok' as const,
  counts: {
    ok: rows.length,
    degraded: 0,
    unreachable: 0,
    blocked: 0,
    carryingTraffic: 0,
    unknown: 0,
  },
  rttMs: 42,
  cadenceSecs: which === 'domestic' || which === 'foreign' ? 5 : 45,
  windowSecs: 60,
  rows,
});

const snapshot = (overrides: Partial<NetworkSnapshot> = {}): NetworkSnapshot => ({
  uptimeSecs: 90,
  timelinePoints: 24,
  diagnosis: {
    verdict: 'clear',
    actionable: false,
    endpointsAffected: 0,
    endpointsTotal: 0,
  },
  sections: [
    section('domestic', [row({ key: 'ru/yandex', label: 'Yandex DNS' })]),
    section('foreign', [row()]),
    section('gamingPlatform', [row({ key: 'services/ea', label: 'EA' })]),
    section('infrastructure', [row({ key: 'services/aws', label: 'Amazon Web Services' })]),
  ],
  ...overrides,
});

/** Captures the callback the page subscribes with so a test can push a snapshot. */
const captureEmitter = () => {
  let emit: ((snapshot: NetworkSnapshot) => void) | undefined;
  subscribeToNetwork.mockImplementation((onSnapshot: (s: NetworkSnapshot) => void) => {
    emit = onSnapshot;
    return Promise.resolve(() => undefined);
  });
  return () => emit;
};

/** Renders the page with one snapshot already delivered. */
const show = (view: NetworkSnapshot = snapshot()) => {
  const emitter = captureEmitter();
  render(<NetworkPage />);
  act(() => {
    emitter()?.(view);
  });
};

describe('NetworkPage', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchCoreStatus.mockResolvedValue(new Promise(() => undefined));
    subscribeToHeartbeat.mockResolvedValue(() => undefined);
    subscribeToNetwork.mockResolvedValue(() => undefined);
  });

  it('draws every target with one row component and one history component', () => {
    // The failure the merge exists to end: a baseline target and a gaming platform were one
    // object drawn by two sets of components — `GroupCard`+`TargetRow` beside
    // `ServiceCard`+`EndpointRow` — with two distribution renderings sharing their CSS.
    show();

    expect(document.querySelectorAll('.nm-row')).toHaveLength(4);
    // One history, and it is the strip. `Sparkline` stroked round-trip time in a colour that
    // *stated health*, which is the one rule about colour this product keeps everywhere else.
    expect(document.querySelectorAll('.nm-timeline')).toHaveLength(4);
    expect(document.querySelectorAll('.nm-sparkline')).toHaveLength(0);
  });

  it('lists the four sections in the order the page argues in', () => {
    show();

    const headings = [...document.querySelectorAll('.nm-section__title')].map(
      (node) => node.textContent,
    );
    expect(headings).toEqual(['Domestic', 'Foreign', 'Gaming platforms', 'Infrastructure']);
  });

  it('marks the sections a verdict was drawn from, and only those', () => {
    // What keeps the banner at the top checkable against the rows below it.
    show();

    const marks = screen.getAllByText('Read by the verdict');
    expect(marks).toHaveLength(2);
    for (const mark of marks) {
      const heading = mark.closest('.nm-section')?.querySelector('.nm-section__title');
      expect(['Domestic', 'Foreign']).toContain(heading?.textContent);
    }
  });

  it('names the round trip two ways rather than five', () => {
    // `Ping (RTT)`, `Ping, median` (baseline group), `Ping, median` again (service card),
    // `Ping, last check` and `Ping, mean` were all on one page. Two survive at level one; the
    // which-window qualifiers moved a level down and both keep their explanations.
    show();

    // One per section heading, and one `Ping (RTT)` per row — the row's is the bare figure
    // under the heading's name, which is what a folded row shows.
    expect(screen.getAllByText('Ping, median')).toHaveLength(4);
    // The which-window qualifiers are present in the document — a `<details>` keeps its
    // children — and on screen nowhere until a row is opened.
    for (const name of ['Ping, last check', 'Ping, mean']) {
      for (const found of screen.getAllByText(name)) {
        expect(found).not.toBeVisible();
      }
    }
  });

  it('folds a clean row to one line, and opens it on request', () => {
    // Twenty-three rows each carrying a strip and three figures per endpoint turns "which of
    // these is red" into a scrolling task.
    show();

    const steam = screen.getByText('Steam').closest('details');
    if (!(steam instanceof HTMLDetailsElement)) throw new Error('Steam is not a folding row');
    expect(steam).not.toHaveAttribute('open');
    // Closed: a name, a state, a strip, a round trip. Nothing else.
    expect(within(steam).queryByText('api.steampowered.com')).not.toBeVisible();

    steam.open = true;
    expect(within(steam).getByText('api.steampowered.com')).toBeVisible();
    expect(within(steam).getByText('Ping, last check')).toBeVisible();
  });

  it('opens a row that is worse than clean, without being asked', () => {
    show(
      snapshot({
        sections: [
          section('domestic', [row({ label: 'Yandex DNS' })]),
          section('foreign', [row({ label: 'Steam', health: 'unreachable' })]),
          section('gamingPlatform', []),
          section('infrastructure', []),
        ],
      }),
    );

    expect(screen.getByText('Steam').closest('details')).toHaveAttribute('open');
    expect(screen.getByText('Yandex DNS').closest('details')).not.toHaveAttribute('open');
  });

  it('states the cadence of each section rather than one claim for the page', () => {
    // The cadence difference stopped being two subsystems and became a field on a target.
    show();

    const cadences = [...document.querySelectorAll('.nm-section__cadence p')].map(
      (node) => node.textContent,
    );
    expect(cadences[0]).toContain('every 5 s');
    expect(cadences[2]).toContain('every 45 s');
  });

  it('carries one legend for the whole page rather than one vocabulary per half', () => {
    show();

    expect(screen.getAllByRole('list', { name: 'What a cell can say' })).toHaveLength(1);
  });

  it('shows a count chip that is not a state badge', () => {
    // "This section is degraded" and "two of its targets are degraded" are different claims.
    show();

    expect(countChips(document).length).toBeGreaterThan(0);
    expect(stateBadges(document).length).toBeGreaterThan(0);
    expect(document.querySelectorAll('.nm-count.nm-health')).toHaveLength(0);
  });

  it('says the core stopped rather than showing a page that looks calm', () => {
    subscribeToNetwork.mockRejectedValue(new Error('no channel'));
    render(<NetworkPage />);

    expect(screen.getByText('Measuring the network…')).toBeInTheDocument();
  });
});
