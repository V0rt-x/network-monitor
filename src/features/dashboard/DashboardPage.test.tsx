import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { DashboardPage } from './DashboardPage';
import type { NetworkHealth } from '../../shared/ipc';

const { fetchCoreStatus, subscribeToHeartbeat, subscribeToNetworkHealth } = vi.hoisted(() => ({
  fetchCoreStatus: vi.fn(),
  subscribeToHeartbeat: vi.fn(),
  subscribeToNetworkHealth: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({
  fetchCoreStatus,
  subscribeToHeartbeat,
  subscribeToNetworkHealth,
}));

vi.mock('./Sparkline', () => ({ Sparkline: () => <div data-testid="sparkline" /> }));

const HEALTH: NetworkHealth = {
  uptimeSecs: 30,
  windowSecs: 60,
  groups: [
    {
      group: 'domestic',
      verdict: 'ok',
      counts: { ok: 2, degraded: 0, unreachable: 0, blocked: 0, unknown: 0 },
      rttMs: 8,
      jitterMs: 1,
      lossPct: 0,
      targets: [],
    },
    {
      group: 'foreign',
      verdict: 'unreachable',
      counts: { ok: 0, degraded: 0, unreachable: 2, blocked: 0, unknown: 0 },
      rttMs: null,
      jitterMs: null,
      lossPct: 100,
      targets: [],
    },
  ],
};

/** Captures the callback the page subscribes with so a test can push a snapshot. */
const captureHealthEmitter = () => {
  let emit: ((health: NetworkHealth) => void) | undefined;
  subscribeToNetworkHealth.mockImplementation((onHealth: (health: NetworkHealth) => void) => {
    emit = onHealth;
    return Promise.resolve(() => undefined);
  });
  return () => emit;
};

describe('DashboardPage', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchCoreStatus.mockResolvedValue({
      coreVersion: '0.1.0',
      platform: 'windows',
      readiness: 'ready',
    });
    subscribeToHeartbeat.mockResolvedValue(() => undefined);
    subscribeToNetworkHealth.mockResolvedValue(() => undefined);
  });

  it('says it is still measuring rather than showing an empty page', async () => {
    render(<DashboardPage />);
    expect(screen.getByText('Measuring the network…')).toBeInTheDocument();
    // Let the core-status panel settle so its state update lands inside this test rather
    // than leaking into the next one.
    await screen.findByText('0.1.0');
  });

  it('shows both baselines side by side once a snapshot arrives', async () => {
    const emitter = captureHealthEmitter();
    render(<DashboardPage />);

    const emit = emitter();
    expect(emit).toBeDefined();
    act(() => {
      emit?.(HEALTH);
    });

    // The comparison is the diagnosis: the domestic side is fine, the way out is not.
    expect(await screen.findByRole('heading', { name: 'Domestic' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Foreign' })).toBeInTheDocument();
    expect(screen.getByText('OK')).toBeInTheDocument();
    expect(screen.getByText('Unreachable')).toBeInTheDocument();
    expect(screen.getByText('Figures cover the last 60 s')).toBeInTheDocument();
  });

  it('reports a dead event channel instead of pretending to still be measuring', async () => {
    subscribeToNetworkHealth.mockRejectedValue(new Error('no listener'));
    render(<DashboardPage />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The core stopped sending measurements',
    );
  });
});
