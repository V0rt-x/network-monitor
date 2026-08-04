import { act, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { countChips, stateBadges } from '../../shared/testing';
import type { NetworkRowView, NetworkSnapshot, RowEndpointView, Section } from '../../shared/ipc';
import { NetworkPage } from './NetworkPage';

const {
  subscribeToNetwork,
  fetchCoreStatus,
  subscribeToHeartbeat,
  fetchNetworkCatalogue,
  fetchSettings,
  storeSettings,
} = vi.hoisted(() => ({
  subscribeToNetwork: vi.fn(),
  fetchCoreStatus: vi.fn(),
  subscribeToHeartbeat: vi.fn(),
  fetchNetworkCatalogue: vi.fn(),
  fetchSettings: vi.fn(),
  storeSettings: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({
  subscribeToNetwork,
  fetchCoreStatus,
  subscribeToHeartbeat,
  fetchNetworkCatalogue,
  fetchSettings,
  storeSettings,
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
    // The catalogue editor's own hooks fetch as soon as the page mounts — folded or not,
    // exactly as the application picker's list does — so every test needs an answer for
    // them even when it never opens the editor.
    fetchNetworkCatalogue.mockResolvedValue(new Promise(() => undefined));
    fetchSettings.mockResolvedValue(new Promise(() => undefined));
  });

  it('draws every user-facing tile with one tile component and one history component', () => {
    // The failure the merge exists to end: a baseline target and a gaming platform were one
    // object drawn by two sets of components — `GroupCard`+`TargetRow` beside
    // `ServiceCard`+`EndpointRow` — with two distribution renderings sharing their CSS. The
    // same component now draws `Domestic` and `Foreign` too, inside the verdict's own
    // evidence expander, so all four rows in the fixture still draw with it.
    show();

    expect(document.querySelectorAll('.nm-tile')).toHaveLength(4);
    // One history, and it is the strip. `Sparkline` stroked round-trip time in a colour that
    // *stated health*, which is the one rule about colour this product keeps everywhere else.
    expect(document.querySelectorAll('.nm-timeline')).toHaveLength(4);
    expect(document.querySelectorAll('.nm-sparkline')).toHaveLength(0);
  });

  it('draws only the users own services as tiles on the page, in the order the plan lists', () => {
    // `Domestic` and `Foreign` are not services and are not the user's to remove — they moved
    // one level down, into the verdict banner's own expander, in Phase 6.8.
    show();

    const headings = [...document.querySelectorAll('.nm-section__title')].map(
      (node) => node.textContent,
    );
    expect(headings).toEqual(['Gaming platforms', 'Infrastructure']);
  });

  it('folds the verdict evidence away until asked, and it is where the baselines live', () => {
    show();

    const evidence = screen.getByText('What this is drawn from').closest('details');
    if (!(evidence instanceof HTMLDetailsElement)) {
      throw new Error('the evidence is not a folding element');
    }
    expect(evidence).not.toHaveAttribute('open');

    // `Domestic` and `Foreign` exist, inside the closed expander — a `<details>` keeps its
    // children — and it is the only place on the page they are drawn at all.
    expect(within(evidence).getByText('Domestic')).toBeInTheDocument();
    expect(within(evidence).getByText('Foreign')).toBeInTheDocument();
    expect(within(evidence).getByText('Yandex DNS')).toBeInTheDocument();

    evidence.open = true;
    expect(within(evidence).getByText('Yandex DNS')).toBeVisible();
  });

  it('names the round trip two ways rather than five', () => {
    // `Ping (RTT)`, `Ping, median` (baseline group), `Ping, median` again (service card),
    // `Ping, last check` and `Ping, mean` were all on one page. Two survive at level one; the
    // which-window qualifiers moved a level down and both keep their explanations.
    show();

    // One per section heading (two tile sections, two evidence sections) and one
    // `Ping (RTT)` per row — the row's is the bare figure under the heading's name, which is
    // what a folded row shows.
    expect(screen.getAllByText('Ping, median')).toHaveLength(4);
    // The which-window qualifiers are present in the document — a `<details>` keeps its
    // children — and on screen nowhere until a tile is opened.
    for (const name of ['Ping, last check', 'Ping, mean']) {
      for (const found of screen.getAllByText(name)) {
        expect(found).not.toBeVisible();
      }
    }
  });

  it('folds a clean tile to one line, and opens it on request', () => {
    // Twenty-three rows each carrying a strip and three figures per endpoint turns "which of
    // these is red" into a scrolling task.
    show();

    const ea = screen.getByText('EA').closest('details');
    if (!(ea instanceof HTMLDetailsElement)) throw new Error('EA is not a folding tile');
    expect(ea).not.toHaveAttribute('open');
    // Closed: a name, a state, a strip, a round trip. Nothing else.
    expect(within(ea).queryByText('api.steampowered.com')).not.toBeVisible();

    ea.open = true;
    expect(within(ea).getByText('api.steampowered.com')).toBeVisible();
    expect(within(ea).getByText('Ping, last check')).toBeVisible();
  });

  it('opens a tile that is worse than clean, without being asked', () => {
    show(
      snapshot({
        sections: [
          section('domestic', [row({ label: 'Yandex DNS' })]),
          section('foreign', [row({ label: 'Steam' })]),
          section('gamingPlatform', [
            row({ key: 'services/ea', label: 'EA', health: 'unreachable' }),
          ]),
          section('infrastructure', []),
        ],
      }),
    );

    expect(screen.getByText('EA').closest('details')).toHaveAttribute('open');
  });

  it('draws no heading at all for a group with nothing selected', () => {
    // An empty `Infrastructure` heading with no rows under it would read as "nothing here is
    // reachable" about a group the user simply unticked — item 4's rule, reused for the third
    // group item 18 added.
    show(
      snapshot({
        sections: [
          section('domestic', [row({ label: 'Yandex DNS' })]),
          section('foreign', [row({ label: 'Steam' })]),
          section('gamingPlatform', [row({ key: 'services/ea', label: 'EA' })]),
          section('infrastructure', []),
          section('other', []),
        ],
      }),
    );

    const headings = [...document.querySelectorAll('.nm-section__title')].map(
      (node) => node.textContent,
    );
    expect(headings).toEqual(['Gaming platforms']);
  });

  it('states the cadence of each tile section rather than one claim for the page', () => {
    // The cadence difference stopped being two subsystems and became a field on a target.
    show();

    const cadences = [...document.querySelectorAll('.nm-section__cadence p')].map(
      (node) => node.textContent,
    );
    expect(cadences[0]).toContain('every 45 s');
  });

  it('carries no page-level legend, because that explanation moved into the help', () => {
    // The heading, the six named marks and the caveat paragraph were an explanation sitting
    // above the page's own content — level one carrying prose, which the standing rule
    // forbids. It is a help topic now, reachable from the general Help page rather than
    // repeated here.
    show();

    expect(document.querySelector('.nm-status__legend')).toBeNull();
    expect(screen.queryByRole('list', { name: 'What a cell can say' })).not.toBeInTheDocument();
  });

  it('shows a count chip that is not a state badge', () => {
    // "This section is degraded" and "two of its targets are degraded" are different claims.
    show();

    expect(countChips(document).length).toBeGreaterThan(0);
    expect(stateBadges(document).length).toBeGreaterThan(0);
    // And a chip is not shaped like one: the colour is a class of its own, so
    // never picks up a pill's border by sharing it.
    expect(document.querySelectorAll('.nm-count.nm-health')).toHaveLength(0);
    expect(document.querySelectorAll('.nm-count.nm-token')).toHaveLength(0);
  });

  it('carries a tile-level chip only where its endpoints disagree', () => {
    show(
      snapshot({
        sections: [
          section('domestic', [row({ label: 'Yandex DNS' })]),
          section('foreign', [row({ label: 'Steam' })]),
          section('gamingPlatform', [
            row({
              key: 'services/ea',
              label: 'EA',
              // The row's own verdict stays clean here on purpose, so the tile starts
              // folded and this test isolates the *summary* chip rather than the one the
              // opened detail always shows for a multi-endpoint row.
              health: 'ok',
              counts: {
                ok: 1,
                degraded: 1,
                unreachable: 0,
                blocked: 0,
                carryingTraffic: 0,
                unknown: 0,
              },
              endpoints: [
                endpoint({ key: 'a', health: 'ok' }),
                endpoint({ key: 'b', health: 'degraded' }),
              ],
            }),
          ]),
          section('infrastructure', [row({ key: 'services/aws', label: 'Amazon Web Services' })]),
        ],
      }),
    );

    // `.nm-tile__chips` is the summary-level chip specifically — the detail below carries an
    // unmarked distribution of its own whenever a row has more than one endpoint, open or
    // not, and the two must not be confused with each other by this assertion.
    const ea = screen.getByText('EA').closest('.nm-tile');
    if (ea === null) throw new Error('EA is not a tile');
    expect(ea.querySelectorAll('.nm-tile__chips')).toHaveLength(1);

    const aws = screen.getByText('Amazon Web Services').closest('.nm-tile');
    if (aws === null) throw new Error('Amazon Web Services is not a tile');
    expect(aws.querySelectorAll('.nm-tile__chips')).toHaveLength(0);
  });

  it('offers an edit control, folded by default', () => {
    show();

    expect(screen.getByRole('button', { name: 'Edit…' })).toBeInTheDocument();
    // Folded: no checklist is on screen until it is asked for.
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
  });

  it('says the core stopped rather than showing a page that looks calm', () => {
    subscribeToNetwork.mockRejectedValue(new Error('no channel'));
    render(<NetworkPage />);

    expect(screen.getByText('Measuring the network…')).toBeInTheDocument();
  });
});
