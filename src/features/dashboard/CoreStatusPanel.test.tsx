import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { CoreStatusPanel } from './CoreStatusPanel';
import type { CoreHeartbeat, CoreStatus } from '../../shared/ipc';

const { fetchCoreStatus, subscribeToHeartbeat } = vi.hoisted(() => ({
  fetchCoreStatus: vi.fn(),
  subscribeToHeartbeat: vi.fn(),
}));

vi.mock('../../shared/ipc', () => ({ fetchCoreStatus, subscribeToHeartbeat }));

const READY: CoreStatus = {
  coreVersion: '0.1.0',
  platform: 'windows',
  readiness: 'ready',
};

/** Captures the callback the panel subscribes with so tests can push a beat. */
const captureHeartbeatEmitter = () => {
  let emit: ((beat: CoreHeartbeat) => void) | undefined;
  subscribeToHeartbeat.mockImplementation((onBeat: (beat: CoreHeartbeat) => void) => {
    emit = onBeat;
    return Promise.resolve(() => undefined);
  });
  return () => emit;
};

describe('CoreStatusPanel', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    subscribeToHeartbeat.mockResolvedValue(() => undefined);
  });

  it('shows the facts reported by the Rust core', async () => {
    fetchCoreStatus.mockResolvedValue(READY);

    render(<CoreStatusPanel />);

    expect(await screen.findByText('0.1.0')).toBeInTheDocument();
    expect(screen.getByText('Windows — Ready')).toBeInTheDocument();
    expect(screen.getByText('Waiting for the first heartbeat…')).toBeInTheDocument();
  });

  it('renders an unsupported platform as a degraded state rather than hiding it', async () => {
    fetchCoreStatus.mockResolvedValue({
      coreVersion: '0.1.0',
      platform: 'unsupported',
      readiness: 'unsupportedPlatform',
    } satisfies CoreStatus);

    render(<CoreStatusPanel />);

    const fact = await screen.findByText('Unsupported — No backend for this operating system');
    expect(fact).toHaveClass('nm-state--degraded');
    // The version we do know must still be shown.
    expect(screen.getByText('0.1.0')).toBeInTheDocument();
  });

  it('reports an unreachable core instead of rendering an empty panel', async () => {
    fetchCoreStatus.mockRejectedValue(new Error('ipc down'));

    render(<CoreStatusPanel />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The monitoring core did not respond',
    );
  });

  it('renders uptime from the heartbeat event', async () => {
    fetchCoreStatus.mockResolvedValue(READY);
    const emitter = captureHeartbeatEmitter();

    render(<CoreStatusPanel />);
    await screen.findByText('0.1.0');

    const emit = emitter();
    expect(emit).toBeDefined();
    act(() => {
      emit?.({ seq: 42, uptimeSecs: 1234 });
    });

    expect(await screen.findByText(/1,234 s/)).toBeInTheDocument();
    expect(screen.getByText('tick 42')).toBeInTheDocument();
  });

  it('surfaces a broken event channel', async () => {
    fetchCoreStatus.mockResolvedValue(READY);
    subscribeToHeartbeat.mockRejectedValue(new Error('no listener'));

    render(<CoreStatusPanel />);

    expect(await screen.findByText('The core event channel is not available')).toBeInTheDocument();
  });
});
