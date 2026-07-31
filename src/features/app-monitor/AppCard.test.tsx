import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { AppCard } from './AppCard';
import type { AppView, EndpointView } from '../../shared/ipc';

// uPlot draws to a canvas, which jsdom does not implement. The chart carries no information
// the row does not also state in text, so the tests replace it outright.
vi.mock('../dashboard/Sparkline', () => ({
  Sparkline: ({ label }: { label: string }) => <div data-testid="sparkline" aria-label={label} />,
}));

const endpoint = (overrides: Partial<EndpointView> = {}): EndpointView => ({
  key: 'udp/1.1.1.1:27015',
  address: '1.1.1.1:27015',
  transport: 'udp',
  health: 'ok',
  liveness: 'active',
  probing: 'active',
  recentBytes: 4096,
  egress: '192.0.2.10',
  egressConflict: false,
  tunnelled: false,
  measurable: true,
  probeKind: 'icmpEcho',
  filteringConfirmed: false,
  rttMs: 24,
  jitterMs: 3,
  lossPct: 0,
  seriesAgeSecs: [-2, -1, 0],
  seriesRttMs: [24, null, 25],
  ...overrides,
});

const app = (overrides: Partial<AppView> = {}): AppView => ({
  pid: 4242,
  name: 'game.exe',
  counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  endpoints: [endpoint()],
  ...overrides,
});

describe('AppCard', () => {
  it('names the process it is following', () => {
    render(<AppCard app={app()} trafficWindowSecs={30} onForget={vi.fn()} />);

    expect(screen.getByRole('heading', { name: 'game.exe' })).toBeInTheDocument();
    expect(screen.getByText('PID 4242')).toBeInTheDocument();
  });

  it('shows a distribution and never one verdict for the whole application', () => {
    // The rule Phase 4 exists to honour: an app rolled up to its worst endpoint reads as
    // "the game is broken" when the game is fine.
    render(
      <AppCard
        app={app({
          counts: {
            ok: 4,
            degraded: 2,
            unreachable: 1,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 0,
          },
          endpoints: [
            endpoint({ key: 'a', address: '1.1.1.1:1', health: 'unreachable' }),
            endpoint({ key: 'b', address: '1.1.1.2:2', health: 'degraded' }),
            endpoint({ key: 'c', address: '1.1.1.3:3', health: 'ok' }),
          ],
        })}
        trafficWindowSecs={30}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('1 Unreachable')).toBeInTheDocument();
    expect(screen.getByText('2 Degraded')).toBeInTheDocument();
    expect(screen.getByText('4 OK')).toBeInTheDocument();
  });

  it('renders each endpoint with its own state', () => {
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({ key: 'a', address: '1.1.1.1:27015', health: 'unreachable' }),
            endpoint({ key: 'b', address: '1.1.1.2:443', health: 'ok' }),
          ],
        })}
        trafficWindowSecs={30}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('1.1.1.1:27015')).toBeInTheDocument();
    expect(screen.getByText('1.1.1.2:443')).toBeInTheDocument();
    expect(screen.getByText('Unreachable')).toBeInTheDocument();
    expect(screen.getByText('OK')).toBeInTheDocument();
  });

  it('states what an endpoint could not be measured with rather than showing a bare number', () => {
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              tunnelled: true,
              probeKind: 'tlsHello',
              filteringConfirmed: true,
              egressConflict: true,
            }),
          ],
        })}
        trafficWindowSecs={30}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('Through a tunnel')).toBeInTheDocument();
    expect(screen.getByText('TLS')).toBeInTheDocument();
    expect(screen.getByText('Filtering confirmed')).toBeInTheDocument();
    expect(screen.getByText("Route differs from another app's")).toBeInTheDocument();
    expect(screen.getByText('Probes leave from 192.0.2.10')).toBeInTheDocument();
  });

  it('shows a live game server as carrying traffic, never as unreachable', () => {
    // The state a UDP match server is normally in: nothing listens on a game port but the
    // game, so no probe answers, while the traffic proves the server is fine.
    render(
      <AppCard
        app={app({
          counts: {
            ok: 0,
            degraded: 0,
            unreachable: 0,
            blocked: 0,
            carryingTraffic: 1,
            unknown: 0,
          },
          endpoints: [
            endpoint({
              health: 'carryingTraffic',
              recentBytes: 630_000,
              rttMs: null,
              jitterMs: null,
              lossPct: null,
            }),
          ],
        })}
        trafficWindowSecs={30}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getAllByText('Carrying traffic').length).toBeGreaterThan(0);
    expect(screen.queryByText('Unreachable')).not.toBeInTheDocument();
    // It claims nothing it did not measure.
    const rtt = screen.getByText('RTT').parentElement;
    expect(rtt).toHaveTextContent('—');
    expect(screen.getByText('Traffic (30 s)').parentElement).toHaveTextContent('630 kB');
  });

  it('shows a dash rather than a zero where nothing counted the traffic', () => {
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ recentBytes: null, rttMs: null, lossPct: null })] })}
        trafficWindowSecs={30}
        onForget={vi.fn()}
      />,
    );

    const traffic = screen.getByText('Traffic (30 s)').parentElement;
    expect(traffic).toHaveTextContent('—');
    expect(screen.queryByText('0 B')).not.toBeInTheDocument();
  });

  it('says an application has nothing discovered instead of rendering an empty list', () => {
    render(
      <AppCard
        app={app({
          counts: {
            ok: 0,
            degraded: 0,
            unreachable: 0,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 0,
          },
          endpoints: [],
        })}
        trafficWindowSecs={30}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('Nothing discovered yet for this application')).toBeInTheDocument();
  });

  it('lets the user stop following the process', async () => {
    const onForget = vi.fn();
    render(<AppCard app={app()} trafficWindowSecs={30} onForget={onForget} />);

    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));

    expect(onForget).toHaveBeenCalledWith(4242);
  });
});
