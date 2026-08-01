import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { AppMonitorPage } from './AppMonitorPage';
import type { AppEndpoints } from '../../shared/ipc';

const { fetchApplications, forgetApp, monitorApp, subscribeToAppEndpoints } = vi.hoisted(() => ({
  fetchApplications: vi.fn(),
  forgetApp: vi.fn(),
  monitorApp: vi.fn(),
  subscribeToAppEndpoints: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({
  fetchApplications,
  forgetApp,
  monitorApp,
  subscribeToAppEndpoints,
}));

vi.mock('../dashboard/Sparkline', () => ({ Sparkline: () => <div data-testid="sparkline" /> }));

const ENDPOINTS: AppEndpoints = {
  windowSecs: 60,
  trafficWindowSecs: 30,
  chartStepSecs: 3,
  flowStatus: 'active',
  apps: [
    {
      id: 1,
      name: 'game.exe',
      processes: [{ pid: 4242, name: 'game.exe' }],
      counts: { ok: 1, degraded: 0, unreachable: 1, blocked: 0, carryingTraffic: 0, unknown: 0 },
      diagnosis: {
        verdict: 'routeToThisApplication',
        actionable: true,
        endpointsAffected: 1,
        endpointsTotal: 2,
      },
      pool: null,
      chartAgeSecs: [],
      endpoints: [
        {
          key: 'udp/1.1.1.1:27015',
          address: '1.1.1.1:27015',
          transport: 'udp',
          health: 'unreachable',
          liveness: 'active',
          probing: 'active',
          recentBytes: null,
          egress: null,
          egressInterface: null,
          probeEgress: null,
          probeEgressInterface: null,
          egressConflict: false,
          tunnelled: false,
          measurable: true,
          probeKind: 'icmpEcho',
          filteringConfirmed: false,
          rttMs: null,
          jitterMs: null,
          lossPct: 100,
          path: null,
          flow: null,
          passiveRtt: null,
          chartRttMs: [],
          chartPathMs: [],
        },
      ],
    },
  ],
};

/** Captures the callback the page subscribes with so a test can push a snapshot. */
const captureEmitter = () => {
  let emit: ((endpoints: AppEndpoints) => void) | undefined;
  subscribeToAppEndpoints.mockImplementation((onEndpoints: (e: AppEndpoints) => void) => {
    emit = onEndpoints;
    return Promise.resolve(() => undefined);
  });
  return () => emit;
};

describe('AppMonitorPage', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    fetchApplications.mockResolvedValue({ applications: [], problem: null });
    subscribeToAppEndpoints.mockResolvedValue(() => undefined);
  });

  it('waits for the core rather than showing an empty page', async () => {
    render(<AppMonitorPage />);
    expect(screen.getByText('Waiting for the core…')).toBeInTheDocument();
    // Let the picker settle so its state update lands inside this test.
    await screen.findByText('Running applications');
  });

  it('says nothing is chosen once the core answers with no applications', async () => {
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    act(() => {
      emit?.({ ...ENDPOINTS, apps: [] });
    });

    expect(await screen.findByText(/No application is being monitored/)).toBeInTheDocument();
  });

  it('shows each monitored application with its endpoints', async () => {
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    act(() => {
      emit?.(ENDPOINTS);
    });

    expect(await screen.findByRole('heading', { name: 'game.exe' })).toBeInTheDocument();
    expect(screen.getByText('1.1.1.1:27015')).toBeInTheDocument();
    expect(screen.getByText('Figures cover the last 60 s')).toBeInTheDocument();
  });

  it('marks every process an application holds, not only the one that was picked', async () => {
    // An application adopts its namesakes and its children, so the picker has to answer
    // "is this taken" for processes the user never clicked.
    fetchApplications.mockResolvedValue({
      applications: [
        { key: 'game.exe', label: 'game.exe', seedPid: 4242, pids: [4242] },
        { key: 'title.exe', label: 'title.exe', seedPid: 4300, pids: [4300] },
        { key: 'unrelated.exe', label: 'unrelated.exe', seedPid: 900, pids: [900] },
      ],
      problem: null,
    });
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    act(() => {
      emit?.({
        ...ENDPOINTS,
        apps: ENDPOINTS.apps.map((app) => ({
          ...app,
          processes: [
            { pid: 4242, name: 'game.exe' },
            { pid: 4300, name: 'title.exe' },
          ],
        })),
      });
    });

    expect(await screen.findAllByText('Part of game.exe')).toHaveLength(2);
    // And the one nothing claimed can still be chosen.
    expect(screen.getAllByRole('button', { name: 'Monitor' })).toHaveLength(1);
  });

  it('explains a missing tracing session instead of showing an application as quiet', async () => {
    // The default state on Windows until the user performs the one-time setup: no UDP
    // endpoints and no traffic counters anywhere. An empty list must not be read as "this
    // game is not talking to anything".
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    act(() => {
      emit?.({ ...ENDPOINTS, flowStatus: 'notPermitted', apps: [] });
    });

    expect(await screen.findByRole('status')).toHaveTextContent(
      /UDP endpoints and traffic counters are missing/,
    );
  });

  it('says nothing about flow events while they are running', async () => {
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    act(() => {
      emit?.(ENDPOINTS);
    });

    await screen.findByRole('heading', { name: 'game.exe' });
    // The flow banner specifically, not every live region: an application's verdict is
    // also announced, and it is announced whatever the tracing session is doing.
    expect(
      screen
        .queryAllByRole('status')
        .filter((element) => element.classList.contains('nm-apps__flow')),
    ).toHaveLength(0);
  });

  it('reports a dead event channel instead of looking like nothing was chosen', async () => {
    subscribeToAppEndpoints.mockRejectedValue(new Error('no listener'));
    render(<AppMonitorPage />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The core stopped sending application measurements',
    );
  });
});
