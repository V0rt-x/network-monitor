import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { AppCard } from './AppCard';
import type { AppView, EndpointView } from '../../shared/ipc';

// uPlot draws to a canvas, which jsdom does not implement. The chart is additive — every
// figure it draws is also stated in the list beside it — so the tests replace it with a
// stand-in that records what it was asked to draw.
vi.mock('./EndpointChart', () => ({
  EndpointChart: ({
    lines,
    label,
  }: {
    lines: { label: string; isPath: boolean }[];
    label: string;
  }) => (
    <div data-testid="chart" aria-label={label}>
      {lines.map((line) => (
        // In an attribute rather than as text: the addresses are already on the page, in the
        // list, and a stand-in that repeated them would make every query ambiguous.
        <span
          key={line.label}
          data-testid="chart-line"
          data-label={line.label}
          data-path={String(line.isPath)}
        />
      ))}
    </div>
  ),
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
  egressInterface: 'Ethernet',
  probeEgress: null,
  probeEgressInterface: null,
  egressConflict: false,
  tunnelled: false,
  measurable: true,
  probeKind: 'icmpEcho',
  filteringConfirmed: false,
  rttMs: 24,
  jitterMs: 3,
  lossPct: 0,
  path: null,
  flow: null,
  passiveRtt: null,
  chartRttMs: [24, null, 25],
  chartPathMs: [null, null, null],
  ...overrides,
});

const app = (overrides: Partial<AppView> = {}): AppView => ({
  id: 1,
  name: 'game.exe',
  processes: [{ pid: 4242, name: 'game.exe' }],
  counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  chartAgeSecs: [-2, -1, 0],
  endpoints: [endpoint()],
  ...overrides,
});

describe('AppCard', () => {
  it('names the application it is following', () => {
    render(<AppCard app={app()} trafficWindowSecs={30} chartStepSecs={3} onForget={vi.fn()} />);

    expect(screen.getByRole('heading', { name: 'game.exe' })).toBeInTheDocument();
    expect(screen.getByText('game.exe · PID 4242')).toBeInTheDocument();
  });

  it('lists every process the application currently consists of', () => {
    // A grouping the user cannot inspect is one they cannot correct: a launcher, the title
    // it started and an anti-cheat shim are one application, and they can see so.
    render(
      <AppCard
        app={app({
          name: 'Example Game',
          processes: [
            { pid: 100, name: 'launcher.exe' },
            { pid: 300, name: 'title.exe' },
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('launcher.exe · PID 100')).toBeInTheDocument();
    expect(screen.getByText('title.exe · PID 300')).toBeInTheDocument();
  });

  it('says an armed application has nothing running rather than showing an empty list', () => {
    // The user picked it before starting the game. Silence here would read as a bug.
    render(
      <AppCard
        app={app({ processes: [] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/Nothing is running under this application yet/)).toBeInTheDocument();
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
        chartStepSecs={3}
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
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('1.1.1.1:27015')).toBeInTheDocument();
    expect(screen.getByText('1.1.1.2:443')).toBeInTheDocument();
    expect(screen.getByText('Unreachable')).toBeInTheDocument();
    expect(screen.getByText('OK')).toBeInTheDocument();
  });

  it('keeps the path figure beside a silent endpoint rather than standing in for it', () => {
    // A match server answers nothing, so its own round-trip time stays a dash. The route to
    // it is a different quantity, measured against a different machine, and the two must
    // never merge into one number called "ping".
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              health: 'carryingTraffic',
              probeKind: null,
              rttMs: null,
              jitterMs: null,
              lossPct: null,
              path: {
                hopTtl: 12,
                hopsProbed: 3,
                position: 'beyondALongHaulLink',
                quality: 'ok',
                rttMs: 84.2,
                jitterMs: 2.5,
                lossPct: 0,
              },
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('Path to this endpoint')).toBeInTheDocument();
    expect(screen.getByText(/12 hops out/)).toBeInTheDocument();
    // Three dashes for the endpoint itself — round trip, jitter and loss — beside the
    // route's three figures.
    expect(screen.getAllByText('—')).toHaveLength(3);
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
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('Through a tunnel')).toBeInTheDocument();
    expect(screen.getByText('TLS')).toBeInTheDocument();
    expect(screen.getByText('Filtering confirmed')).toBeInTheDocument();
    expect(screen.getByText("Probe may not follow this app's route")).toBeInTheDocument();
    expect(
      screen.getByText(/This app's traffic leaves via Ethernet \(192\.0\.2\.10\)/),
    ).toBeInTheDocument();
  });

  it('names the adapter, not only the address, so a VPN change is recognisable', () => {
    // The core use case: comparing before and after turning an accelerator on. An address
    // is not something a user can check that against; the name they see in Windows is.
    render(
      <AppCard
        app={app({
          endpoints: [endpoint({ egress: '10.7.0.2', egressInterface: 'Accelerator' })],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/leaves via Accelerator \(10\.7\.0\.2\)/)).toBeInTheDocument();
  });

  it('falls back to the bare address when no adapter claims it', () => {
    // A tunnel that went down between the adapter snapshot and this emission. A guessed
    // name would be worse than none.
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ egress: '10.7.0.2', egressInterface: null })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/leaves from 10\.7\.0\.2/)).toBeInTheDocument();
  });

  it('names the route the probe takes when it cannot follow the application', () => {
    // The per-process interceptor case. Saying only "this may be wrong" leaves the user
    // nothing to act on; saying which route the figure describes gives them the answer.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              egress: '10.7.0.2',
              egressInterface: 'Accelerator',
              probeEgress: '192.0.2.10',
              probeEgressInterface: 'Ethernet',
              egressConflict: true,
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/leaves via Accelerator \(10\.7\.0\.2\)/)).toBeInTheDocument();
    expect(
      screen.getByText(/The probe cannot follow it and leaves via Ethernet \(192\.0\.2\.10\)/),
    ).toBeInTheDocument();
    expect(screen.getByText("Probe may not follow this app's route")).toBeInTheDocument();
  });

  it('says nothing about a second route when the probe follows the application', () => {
    render(<AppCard app={app()} trafficWindowSecs={30} chartStepSecs={3} onForget={vi.fn()} />);

    expect(screen.queryByText(/The probe cannot follow it/)).not.toBeInTheDocument();
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
        chartStepSecs={3}
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

  it("shows the operating system's own round trip with its age, never as a live figure", () => {
    // The one genuine round trip that cost no packet — and it arrives every few tens of
    // seconds at best, so a figure without its age would read as current when it is not.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              transport: 'tcp',
              passiveRtt: { rttMs: 24.5, minRttMs: 21, maxRttMs: 90, ageSecs: 37 },
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    const stack = screen.getByText('RTT from the OS (ms)').parentElement;
    // Rounded by the same rule the probes' own figure uses: whole milliseconds above 10.
    expect(stack).toHaveTextContent('25');
    expect(stack).toHaveTextContent('37 s ago');
  });

  it("omits the operating system's round trip where it published none", () => {
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ passiveRtt: null })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.queryByText('RTT from the OS (ms)')).not.toBeInTheDocument();
  });

  it('shows a dash rather than a zero where nothing counted the traffic', () => {
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ recentBytes: null, rttMs: null, lossPct: null })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
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
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('Nothing discovered yet for this application')).toBeInTheDocument();
  });

  it('lets the user stop following the application', async () => {
    // By application identity, never by process: the one the user picked may be long gone
    // while its children are still being measured.
    const onForget = vi.fn();
    render(
      <AppCard app={app({ id: 7 })} trafficWindowSecs={30} chartStepSecs={3} onForget={onForget} />,
    );

    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));

    expect(onForget).toHaveBeenCalledWith(7);
  });

  // ------------------------------------------------------- one chart, every endpoint

  const lines = () =>
    screen.queryAllByTestId('chart-line').map((node) => ({
      label: node.getAttribute('data-label'),
      isPath: node.getAttribute('data-path') === 'true',
    }));

  it('draws every endpoint on one chart', () => {
    // The question the page has to answer during a match is "which of these is the odd one
    // out", and only a shared axis answers it.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({ key: 'a', address: '1.1.1.1:27015' }),
            endpoint({ key: 'b', address: '1.1.1.2:443' }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(lines()).toEqual([
      { label: '1.1.1.1:27015', isPath: false },
      { label: '1.1.1.2:443', isPath: false },
    ]);
  });

  it('puts a silent endpoint on the chart by its route, named as the route', () => {
    // The endpoint the whole product exists to watch has no round trip to draw. Leaving it
    // off would hide it; drawing its path figure as a round trip would be the lie this
    // product was built not to tell.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              key: 'silent',
              address: '1.1.1.9:27015',
              health: 'carryingTraffic',
              chartRttMs: [null, null, null],
              chartPathMs: [80, null, 84],
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(lines()).toEqual([{ label: 'Route to 1.1.1.9:27015', isPath: true }]);
  });

  it('draws both lines for an endpoint that has a round trip and a route', () => {
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({ key: 'both', address: '1.1.1.5:27015', chartPathMs: [10, 11, 12] }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(lines()).toEqual([
      { label: '1.1.1.5:27015', isPath: false },
      { label: 'Route to 1.1.1.5:27015', isPath: true },
    ]);
  });

  it('shows an endpoint with nothing to draw in the list all the same', () => {
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              key: 'quiet',
              address: '1.1.1.7:443',
              health: 'unknown',
              chartRttMs: [null, null, null],
              chartPathMs: [null, null, null],
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    expect(lines()).toEqual([]);
    expect(screen.getByText('1.1.1.7:443')).toBeInTheDocument();
  });

  it('raises one endpoint and dims the rest without hiding either', async () => {
    // The keyboard path to what hovering a line does. Dimmed, never hidden: the endpoint the
    // user is not looking at is still a row they can read.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({ key: 'a', address: '1.1.1.1:27015' }),
            endpoint({ key: 'b', address: '1.1.1.2:443' }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    const [chosen] = screen.getAllByRole('button', { name: /1\.1\.1\.1:27015/ });
    if (!chosen) throw new Error('the row offers no way to raise its endpoint');
    await userEvent.click(chosen);

    expect(chosen).toHaveAttribute('aria-pressed', 'true');
    const rows = screen
      .getAllByRole('listitem')
      .filter((row) => row.className.includes('nm-endpoint'));
    expect(rows[0]?.className).toContain('nm-endpoint--raised');
    expect(rows[1]?.className).toContain('nm-endpoint--dimmed');
    expect(screen.getByText('1.1.1.2:443')).toBeInTheDocument();
  });

  it('lets a pinned endpoint be released again', async () => {
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ key: 'a', address: '1.1.1.1:27015' })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        onForget={vi.fn()}
      />,
    );

    const [chosen] = screen.getAllByRole('button', { name: /1\.1\.1\.1:27015/ });
    if (!chosen) throw new Error('the row offers no way to raise its endpoint');
    await userEvent.click(chosen);
    await userEvent.click(chosen);

    expect(chosen).toHaveAttribute('aria-pressed', 'false');
  });
});
