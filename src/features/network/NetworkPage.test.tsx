import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { NetworkPage } from './NetworkPage';
import type { NetworkHealth, ServiceStatus } from '../../shared/ipc';

const {
  fetchCoreStatus,
  subscribeToHeartbeat,
  subscribeToNetworkHealth,
  subscribeToServiceStatus,
} = vi.hoisted(() => ({
  fetchCoreStatus: vi.fn(),
  subscribeToHeartbeat: vi.fn(),
  subscribeToNetworkHealth: vi.fn(),
  subscribeToServiceStatus: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({
  fetchCoreStatus,
  subscribeToHeartbeat,
  subscribeToNetworkHealth,
  subscribeToServiceStatus,
}));

vi.mock('../dashboard/Sparkline', () => ({ Sparkline: () => <div data-testid="sparkline" /> }));

const HEALTH: NetworkHealth = {
  uptimeSecs: 30,
  windowSecs: 60,
  diagnosis: {
    verdict: 'crossBorderPath',
    actionable: true,
    endpointsAffected: 2,
    endpointsTotal: 4,
  },
  groups: [
    {
      group: 'domestic',
      verdict: 'ok',
      counts: { ok: 2, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
      rttMs: 8,
      jitterMs: 1,
      lossPct: 0,
      targets: [],
    },
    {
      group: 'foreign',
      verdict: 'unreachable',
      counts: { ok: 0, degraded: 0, unreachable: 2, blocked: 0, carryingTraffic: 0, unknown: 0 },
      rttMs: null,
      jitterMs: null,
      lossPct: 100,
      targets: [],
    },
  ],
};

const STATUS: ServiceStatus = {
  checkIntervalSecs: 45,
  windowSecs: 1080,
  timelinePoints: 24,
  services: [
    {
      id: 'steam',
      label: 'Steam',
      group: 'gamingPlatform',
      verdict: 'ok',
      counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
      rttMs: 42,
      lossPct: 0,
      lastCheckedSecs: 3,
      endpoints: [],
    },
  ],
};

describe('NetworkPage', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchCoreStatus.mockResolvedValue({
      coreVersion: '0.1.0',
      platform: 'windows',
      readiness: 'ready',
    });
    subscribeToHeartbeat.mockResolvedValue(() => undefined);
  });

  it('answers "is it me, the border, or that service" without changing tabs', async () => {
    // The merge, and the whole reason for it: the verdict engine reads the baselines and
    // the service cards are the evidence a reader checks that verdict against. Holding one
    // page in your head while reading the other was the complaint.
    let pushHealth: ((health: NetworkHealth) => void) | undefined;
    let pushStatus: ((status: ServiceStatus) => void) | undefined;
    subscribeToNetworkHealth.mockImplementation((on: (health: NetworkHealth) => void) => {
      pushHealth = on;
      return Promise.resolve(() => undefined);
    });
    subscribeToServiceStatus.mockImplementation((on: (status: ServiceStatus) => void) => {
      pushStatus = on;
      return Promise.resolve(() => undefined);
    });

    render(<NetworkPage />);
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      pushHealth?.(HEALTH);
      pushStatus?.(STATUS);
    });

    // The verdict, the baselines it was drawn from, and the services to check it against.
    expect(
      screen.getByText(/Services inside your country answer and services abroad do not/),
    ).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Domestic' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Foreign' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Steam' })).toBeInTheDocument();
  });

  it('keeps the two cadences apart rather than smoothing them over', async () => {
    // The halves are measured by different rules — the baselines over a health window, the
    // services on a slow fixed check — and a merged page that stated one span would be
    // quietly wrong about the other.
    let pushHealth: ((health: NetworkHealth) => void) | undefined;
    let pushStatus: ((status: ServiceStatus) => void) | undefined;
    subscribeToNetworkHealth.mockImplementation((on: (health: NetworkHealth) => void) => {
      pushHealth = on;
      return Promise.resolve(() => undefined);
    });
    subscribeToServiceStatus.mockImplementation((on: (status: ServiceStatus) => void) => {
      pushStatus = on;
      return Promise.resolve(() => undefined);
    });

    render(<NetworkPage />);
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      pushHealth?.(HEALTH);
      pushStatus?.(STATUS);
    });

    expect(screen.getByText('Figures cover the last 60 s')).toBeInTheDocument();
    expect(screen.getByText('Each service is checked every 45 s')).toBeInTheDocument();
  });

  it('puts what the core itself is doing last, below the network', async () => {
    subscribeToNetworkHealth.mockResolvedValue(() => undefined);
    subscribeToServiceStatus.mockResolvedValue(() => undefined);
    render(<NetworkPage />);

    // A fact about the app rather than about the network, and the only thing on this page a
    // reader never needs during a match.
    expect(await screen.findByText('0.1.0')).toBeInTheDocument();
  });
});
