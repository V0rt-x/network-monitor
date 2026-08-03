import { act, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { StatusPage } from './StatusPage';
import type { ServiceStatus, ServiceView } from '../../shared/ipc';

const { subscribeToServiceStatus } = vi.hoisted(() => ({
  subscribeToServiceStatus: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({ subscribeToServiceStatus }));

const service = (overrides: Partial<ServiceView> = {}): ServiceView => ({
  id: 'steam',
  label: 'Steam',
  group: 'gamingPlatform',
  verdict: 'ok',
  counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  rttMs: 42,
  lossPct: 0,
  lastCheckedSecs: 3,
  endpoints: [
    {
      key: 'steam/api',
      writtenAddress: 'api.steampowered.com',
      resolvedAddress: '203.0.113.7:443',
      tunnelled: false,
      measurable: true,
      probeKind: 'tcpConnect',
      filteringConfirmed: false,
      health: 'ok',
      rttMs: 42,
      meanRttMs: 44,
      lossPct: 0,
      checks: [{ ageSecs: 0, mark: 'answered' }],
    },
  ],
  ...overrides,
});

const STATUS: ServiceStatus = {
  checkIntervalSecs: 45,
  windowSecs: 1080,
  timelinePoints: 24,
  services: [
    service(),
    service({
      id: 'cloudflare',
      label: 'Cloudflare',
      group: 'infrastructure',
      verdict: 'unreachable',
      counts: { ok: 0, degraded: 0, unreachable: 1, blocked: 0, carryingTraffic: 0, unknown: 0 },
      rttMs: null,
    }),
  ],
};

/** Hands the page one snapshot through the subscription it opened. */
const emit = async (status: ServiceStatus) => {
  let push: ((status: ServiceStatus) => void) | undefined;
  subscribeToServiceStatus.mockImplementation((onStatus: (status: ServiceStatus) => void) => {
    push = onStatus;
    return Promise.resolve(() => undefined);
  });

  render(<StatusPage />);
  await act(async () => {
    await Promise.resolve();
  });
  act(() => {
    push?.(status);
  });
};

describe('StatusPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('says it is checking before the first snapshot arrives', async () => {
    subscribeToServiceStatus.mockResolvedValue(() => undefined);
    render(<StatusPage />);
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByText('Checking services…')).toBeInTheDocument();
  });

  it('reports a broken event channel rather than an empty page', async () => {
    // An empty status page reads as calm, which is the one thing a lost channel must not
    // be mistaken for.
    subscribeToServiceStatus.mockRejectedValue(new Error('no channel'));
    render(<StatusPage />);
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByRole('alert')).toHaveTextContent('The core stopped sending service checks');
  });

  it('shelves the services under their groups', async () => {
    await emit(STATUS);

    expect(screen.getByRole('heading', { name: 'Gaming platforms' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Infrastructure' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Steam' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Cloudflare' })).toBeInTheDocument();
  });

  it('states the cadence its figures were taken at', async () => {
    await emit(STATUS);
    expect(screen.getByText('Each service is checked every 45 s')).toBeInTheDocument();
  });

  it('states what a failed check does and does not mean', async () => {
    // The page reports what *this machine* can reach. From inside a filtered network that
    // is indistinguishable from an outage, and only one of the two is observable here.
    await emit(STATUS);
    expect(
      screen.getByText(
        'These checks say whether this machine can reach each service. A service you cannot reach may be running normally for everyone else.',
      ),
    ).toBeInTheDocument();
  });

  it('says what one cell is, how far back a strip reaches, and what every colour means', async () => {
    // The user asked what the coloured cells meant, which is a page failing at its one job.
    // All three facts belong on the page rather than in the source, and the legend is also
    // what stops colour being the only channel: every state is named beside its colour.
    await emit(STATUS);

    expect(
      screen.getByText(
        'One check, oldest on the left. A full strip is the last 24 checks — about 18 min, which is also the window every mean and loss below covers.',
      ),
    ).toBeInTheDocument();

    const legend = screen.getByRole('list', { name: 'What a cell can say' });
    for (const state of [
      'No answer',
      'Probe filtered',
      'Your tunnel answered',
      'Refused',
      'Answered slowly',
      'Answered',
    ]) {
      expect(within(legend).getByText(state)).toBeInTheDocument();
    }
  });

  it('says a group is empty rather than rendering a bare heading', async () => {
    await emit({ ...STATUS, services: [service()] });
    expect(screen.getByText('No services in this group')).toBeInTheDocument();
  });
});
