import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
      warmupSecsRemaining: null,
      chartElapsedSecs: [],
      groups: [
        {
          transport: 'udp',
          counts: {
            ok: 0,
            degraded: 0,
            unreachable: 1,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 0,
          },
          needsAttention: true,
          endpoints: [
            {
              key: 'udp/1.1.1.1:27015',
              address: '1.1.1.1:27015',
              network: null,
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
              probesMeasureIt: true,
              probeKind: 'icmpEcho',
              filteringConfirmed: false,
              rttMs: null,
              jitterMs: null,
              lossPct: 100,
              path: null,
              flow: null,
              passiveRtt: null,
              age: { secs: 300, kind: 'watched' },
              warmupSecsRemaining: null,
              chartRttMs: [],
              chartPathMs: [],
            },
          ],
        },
        {
          transport: 'tcp',
          counts: {
            ok: 0,
            degraded: 0,
            unreachable: 0,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 0,
          },
          needsAttention: false,
          endpoints: [],
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
    // And the first-run screen is not shown yet. Waiting for the core is not the same fact
    // as nothing being watched, and telling a returning user their applications were gone
    // for the first second of every launch would be a lie with a very short life.
    expect(screen.queryByText('Pick the game you want to watch')).not.toBeInTheDocument();
    // Let the picker settle so its state update lands inside this test.
    await screen.findByText(/Nothing is being watched yet/);
  });

  it('answers "what do I press" once the core reports nothing being watched', async () => {
    // The first run was an expanded picker and one grey sentence — no heading, no statement
    // of what is about to happen, no primary action. The audience installs this because a
    // game stutters.
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    act(() => {
      emit?.({ ...ENDPOINTS, apps: [] });
    });

    expect(await screen.findByText('Pick the game you want to watch')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Choose an application' })).toBeInTheDocument();
    // And it says the rest of the app is not idle meanwhile, which is what stops an empty
    // page reading as an application that is not working.
    expect(screen.getByText(/Your network is already being measured/)).toBeInTheDocument();
    // The picker is open, because with nothing chosen there is nothing else to do.
    expect(screen.getByText('Choose what to watch')).toBeInTheDocument();
  });

  it('folds the picker away once something is being watched', async () => {
    // It is a setup tool — heading, refresh, hint, scope checkbox, filter and up to forty
    // scrolling rows — and it was mounted permanently above the measurements.
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    act(() => {
      emit?.(ENDPOINTS);
    });

    expect(await screen.findByText(/Watching game.exe · 1 of 5/)).toBeInTheDocument();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
    expect(screen.queryByText('Choose what to watch')).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Change…' }));

    expect(screen.getByRole('searchbox')).toBeInTheDocument();
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
      // Named, so the picker's default filter offers all three: what this test is about is
      // which of them the monitor has already claimed.
      applications: [
        {
          key: 'game.exe',
          label: 'game.exe',
          executable: 'game.exe',
          named: true,
          seedPid: 4242,
          pids: [4242],
        },
        {
          key: 'title.exe',
          label: 'title.exe',
          executable: 'title.exe',
          named: true,
          seedPid: 4300,
          pids: [4300],
        },
        {
          key: 'unrelated.exe',
          label: 'unrelated.exe',
          executable: 'unrelated.exe',
          named: true,
          seedPid: 900,
          pids: [900],
        },
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

    // The picker folds once something is being watched, so this is what a reader who asks
    // to change it sees.
    await userEvent.click(await screen.findByRole('button', { name: 'Change…' }));

    expect(await screen.findAllByText('Part of game.exe')).toHaveLength(2);
    // And the one nothing claimed can still be chosen.
    expect(screen.getAllByRole('button', { name: 'Watch' })).toHaveLength(1);
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

  it('carries no running prose at level one, and every warning it had', async () => {
    // The audit of item 4, pinned in the same spirit as the closed-row test: prose grows
    // back one sentence at a time. What is asserted is not a word count but a list — every
    // paragraph that was competing with a figure is gone from the page and reachable in the
    // help, and everything the standing rule protects as a *warning* is untouched.
    const emitter = captureEmitter();
    render(<AppMonitorPage />);
    const emit = emitter();

    // A machine without the one-time tracing setup, which is the *default* on Windows: no
    // UDP endpoints exist to be found, so the match-traffic group is empty and has to say
    // why rather than read as a game that plays over nothing.
    act(() => {
      emit?.({
        ...ENDPOINTS,
        flowStatus: 'notPermitted',
        apps: ENDPOINTS.apps.map((app) => ({
          ...app,
          groups: app.groups.map((group) =>
            group.transport === 'udp' ? { ...group, endpoints: [] } : group,
          ),
        })),
      });
    });
    await screen.findByRole('heading', { name: 'game.exe' });

    // Moved down a level: the chart's six sentences of drawing decisions, what choosing an
    // application does, and how the passive figures are taken.
    for (const moved of [
      /each point the slowest round trip/,
      /Choosing one monitors the whole application/,
      /Measured from the data your game is already exchanging/,
      /and the moment it does it will appear here/,
    ]) {
      expect(screen.queryByText(moved)).not.toBeInTheDocument();
    }

    // Never demoted, whatever their length: a machine that cannot see UDP at all, and a
    // group that is empty because of it. Both are things the user must act on.
    const [flow] = screen
      .getAllByRole('status')
      .filter((element) => element.classList.contains('nm-apps__flow'));
    expect(flow).toHaveTextContent(/UDP endpoints and traffic counters are missing/);
    expect(screen.getByText(/UDP flows cannot be discovered on this machine/)).toBeVisible();
  });

  it('reports a dead event channel instead of looking like nothing was chosen', async () => {
    subscribeToAppEndpoints.mockRejectedValue(new Error('no listener'));
    render(<AppMonitorPage />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The core stopped sending application measurements',
    );
  });
});
