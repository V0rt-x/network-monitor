import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { AppCard } from './AppCard';
import type { AppView, EndpointGroupView, EndpointView, HealthCountsView } from '../../shared/ipc';

// uPlot draws to a canvas, which jsdom does not implement. The chart is additive — every
// figure it draws is also stated in the list beside it — so the tests replace it with a
// stand-in that records what it was asked to draw.
vi.mock('./EndpointChart', () => ({
  EndpointChart: ({
    lines,
    label,
    onSelect,
  }: {
    lines: { endpoint: string; label: string; isPath: boolean }[];
    label: string;
    onSelect: (endpoint: string) => void;
  }) => (
    <div data-testid="chart" aria-label={label}>
      {lines.map((line) => (
        // In an attribute rather than as text: the addresses are already on the page, in the
        // list, and a stand-in that repeated them would make every query ambiguous.
        <button
          type="button"
          key={line.label}
          data-testid="chart-line"
          data-label={line.label}
          data-path={String(line.isPath)}
          onClick={() => {
            onSelect(line.endpoint);
          }}
        />
      ))}
    </div>
  ),
}));

const endpoint = (overrides: Partial<EndpointView> = {}): EndpointView => ({
  key: 'udp/1.1.1.1:27015',
  address: '1.1.1.1:27015',
  network: null,
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
  probesMeasureIt: true,
  probeKind: 'icmpEcho',
  filteringConfirmed: false,
  rttMs: 24,
  jitterMs: 3,
  lossPct: 0,
  path: null,
  flow: null,
  passiveRtt: null,
  age: { secs: 300, kind: 'watched' },
  warmupSecsRemaining: null,
  chartRttMs: [24, null, 25],
  chartPathMs: [null, null, null],
  ...overrides,
});

const NO_COUNTS: HealthCountsView = {
  ok: 0,
  degraded: 0,
  unreachable: 0,
  blocked: 0,
  carryingTraffic: 0,
  unknown: 0,
};

/**
 * Groups a flat endpoint list the way Rust does: the match traffic first, the supporting
 * connections below, each carrying its own distribution.
 *
 * The tests state endpoints as one list because that is what most of them are about; the
 * grouping is reproduced here rather than written out per case, so no test can accidentally
 * describe a shape Rust never sends.
 */
const groupsOf = (endpoints: readonly EndpointView[]): EndpointGroupView[] =>
  (['udp', 'tcp'] as const).map((transport) => {
    const members = endpoints.filter((candidate) => candidate.transport === transport);
    const counts = { ...NO_COUNTS };
    for (const member of members) counts[member.health] += 1;
    return {
      transport,
      counts,
      needsAttention: members.some((member) => member.health !== 'ok'),
      endpoints: members,
    };
  });

const app = ({
  endpoints = [endpoint()],
  ...overrides
}: Partial<Omit<AppView, 'groups'>> & { endpoints?: EndpointView[] } = {}): AppView => ({
  id: 1,
  name: 'game.exe',
  processes: [{ pid: 4242, name: 'game.exe' }],
  counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  diagnosis: {
    verdict: 'clear',
    actionable: false,
    endpointsAffected: 0,
    endpointsTotal: 1,
  },
  pool: null,
  warmupSecsRemaining: null,
  chartElapsedSecs: [0, 3, 6],
  groups: groupsOf(endpoints),
  ...overrides,
});

describe('AppCard', () => {
  it('names the application it is following', () => {
    render(
      <AppCard
        app={app()}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByRole('heading', { name: 'game.exe' })).toBeInTheDocument();
    expect(screen.getByText('1 process')).toBeInTheDocument();
  });

  it('counts the processes at level one instead of listing them', async () => {
    // A browser or an Electron app contributes a dozen-odd entries, and that many lines of
    // `name · PID` above the figures is a wall the reader has to get past to reach what they
    // came for. The count is the part worth a glance: it says how large a group the rule
    // caught, which is what would look wrong if the grouping were wrong.
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('2 processes')).toBeVisible();
    expect(screen.getByText('launcher.exe · PID 100')).not.toBeVisible();

    // But a grouping the user cannot inspect is still one they cannot correct, so the list
    // is one click away rather than gone: a launcher, the title it started and an anti-cheat
    // shim are one application, and they can still see so.
    await userEvent.click(screen.getByText('2 processes'));

    expect(screen.getByText('launcher.exe · PID 100')).toBeVisible();
    expect(screen.getByText('title.exe · PID 300')).toBeVisible();
  });

  it('says an armed application has nothing running rather than showing an empty list', () => {
    // The user picked it before starting the game. Silence here would read as a bug.
    render(
      <AppCard
        app={app({ processes: [] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    // The application's own tally, not a group's: each transport group carries one too, and
    // they are different statements about different sets of endpoints.
    const total = within(
      screen.getByRole('list', { name: 'Endpoint states across this application' }),
    );
    expect(total.getByText('1 Unreachable')).toBeInTheDocument();
    expect(total.getByText('2 Degraded')).toBeInTheDocument();
    expect(total.getByText('4 OK')).toBeInTheDocument();
  });

  it('groups the endpoints by transport, UDP first, and claims no role from it', () => {
    // During a game the endpoints that decide whether it plays well are usually the UDP
    // flows, and severity alone puts them wherever their health happens to fall — between a
    // launcher's connection and a content network. So UDP leads.
    //
    // What the headings must *not* do is turn that tendency into a claim. They read "Match
    // traffic" and "Supporting connections", which infer a role from a transport: Discord's
    // UDP is voice, a browser's is QUIC, and several games play over TCP, which made
    // "supporting" a lie on the most important row on the page.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({ key: 'a', address: '1.1.1.1:27015', transport: 'udp' }),
            endpoint({ key: 'b', address: '1.1.1.2:443', transport: 'tcp', health: 'degraded' }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    const headings = [...document.querySelectorAll('.nm-endpointgroup__title')].map(
      (node) => node.firstChild?.textContent,
    );
    expect(headings).toEqual(['UDP flows', 'TCP connections']);
    expect(screen.queryByText(/Match traffic/)).not.toBeInTheDocument();
    expect(screen.queryByText(/plays over|not where the game is played/)).not.toBeInTheDocument();

    const supporting = within(
      screen.getByRole('list', { name: 'Endpoint states in TCP connections' }),
    );
    expect(supporting.getByText('1 Degraded')).toBeInTheDocument();
  });

  it('explains a group heading rather than captioning it on the page', async () => {
    // The two sentences that used to sit beside the headings were paragraphs competing with
    // the figures they described, and they were the role claim itself. They are now what the
    // heading's own explanation says, which is one keystroke away and costs no ink.
    render(
      <AppCard
        app={app()}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole('button', { name: 'What UDP flows means' }));

    expect(screen.getByText(/no connection to open or close/)).toBeInTheDocument();
  });

  it('says why there is no match traffic rather than showing an empty group', () => {
    // Without the one-time tracing setup there are no UDP endpoints at all on this machine,
    // and an unexplained empty "match traffic" reads as a game that plays over nothing.
    render(
      <AppCard
        app={app({
          endpoints: [endpoint({ key: 'b', address: '1.1.1.2:443', transport: 'tcp' })],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="notPermitted"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/UDP flows cannot be discovered on this machine/)).toBeInTheDocument();
  });

  it('opens the supporting connections when they already need attention, and only then', () => {
    // TCP is demoted, never hidden: a login service with a filter on it is exactly what
    // "I cannot get into the game" looks like — so a group that needs attention when the
    // card first draws starts open.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({ key: 'b', address: '1.1.1.2:443', transport: 'tcp', health: 'blocked' }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('TCP connections').closest('details')).toHaveAttribute('open');
  });

  it('does not open or close the supporting connections under the reader', () => {
    // `needsAttention` decided the fold on every render, and on a weak link it flips
    // constantly: the section opened and collapsed on its own and moved everything below
    // it. It is the *initial* state now, and a problem arriving afterwards is announced by
    // the distribution in the heading, which a folded group shows too.
    const { rerender } = render(
      <AppCard
        app={app({
          endpoints: [endpoint({ key: 'b', address: '1.1.1.2:443', transport: 'tcp' })],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('TCP connections').closest('details')).not.toHaveAttribute('open');

    rerender(
      <AppCard
        app={app({
          endpoints: [
            endpoint({ key: 'b', address: '1.1.1.2:443', transport: 'tcp', health: 'blocked' }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    // Still folded. The heading says what happened instead, and it says it whether the
    // group is open or shut.
    expect(screen.getByText('TCP connections').closest('details')).not.toHaveAttribute('open');
    expect(
      within(screen.getByRole('list', { name: 'Endpoint states in TCP connections' })).getByText(
        '1 Probe blocked',
      ),
    ).toBeInTheDocument();
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('1.1.1.1:27015')).toBeInTheDocument();
    expect(screen.getByText('1.1.1.2:443')).toBeInTheDocument();
    expect(screen.getByText('Unreachable')).toBeInTheDocument();
    expect(screen.getByText('OK')).toBeInTheDocument();
  });

  it('keeps the path figure beside a silent endpoint rather than standing in for it', () => {
    // A match server answers nothing, so it has no round-trip time of its own at all. The
    // route to it is a different quantity, measured against a different machine, and the two
    // must never merge into one number called "ping".
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              health: 'carryingTraffic',
              probeKind: null,
              probesMeasureIt: false,
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
                hopNetwork: null,
              },
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('The route towards it')).toBeInTheDocument();
    expect(screen.getByText('Round trip to that router')).toBeInTheDocument();
    // No dashes at all where the endpoint's own figures would have been: they are gone,
    // not blank. Three dashes are what the most important row on the page used to show,
    // and three dashes read as a broken tool rather than as an honest absence.
    expect(screen.queryByText('Ping (RTT)')).not.toBeInTheDocument();
    // One jitter on the card, and it belongs to the route — the endpoint's own is gone
    // rather than blank.
    expect(screen.getAllByText('Jitter')).toHaveLength(1);
    expect(screen.queryByText('—')).not.toBeInTheDocument();
    // And nothing explains the gap in prose. The page carries figures and findings; the one
    // line that says why is a disclosure the reader opens when they want it.
    expect(screen.queryByText(/no ping, jitter or loss to show/)).not.toBeInTheDocument();
    // And the card says in as many words why this is not the number the game shows. It is
    // the most important string in the application.
    expect(screen.getByText('Why none of this is the ping your game shows')).toBeInTheDocument();
  });

  it('says why none of this is the game’s ping once, not once per silent endpoint', () => {
    // It is the same three-point disclosure every time, and a game has several silent
    // endpoints — so it appeared once per row, six times, saying the same thing. A reader
    // who has had the explanation does not need it again six rows later.
    const silent = (key: string, address: string): Partial<EndpointView> => ({
      key,
      address,
      health: 'carryingTraffic',
      probeKind: null,
      probesMeasureIt: false,
      rttMs: null,
      jitterMs: null,
      lossPct: null,
      flow: {
        spanSecs: 10,
        sentBytesPerSec: 1280,
        receivedBytesPerSec: 20480,
        updatesPerSec: 20,
        arrivalMeanMs: 50,
        arrivalJitterMs: 1.4,
        arrivalP95Ms: 52,
        arrivalMaxMs: 119,
        stallMs: null,
        receiveShortfallPct: null,
      },
    });

    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint(silent('a', '1.1.1.1:27015')),
            endpoint(silent('b', '1.1.1.2:27015')),
            endpoint(silent('c', '1.1.1.3:27015')),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getAllByText('Why none of this is the ping your game shows')).toHaveLength(1);
  });

  it('keeps the dashes of an endpoint whose figures have not arrived yet', () => {
    // The other half of the same rule, and the one a later refactor flattens: *never* and
    // *not yet* are different answers. A chain still working through probe kinds has a
    // figure coming, so the row waits for it in place rather than changing shape and
    // changing back a second later.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              health: 'unknown',
              probeKind: 'icmpEcho',
              probesMeasureIt: true,
              rttMs: null,
              jitterMs: null,
              lossPct: null,
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('Ping (RTT)')).toBeVisible();
    expect(screen.getAllByText('—')).toHaveLength(3);
    expect(screen.queryByText(/no ping, jitter or loss to show/)).not.toBeInTheDocument();
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    // The caveats live a level down, where they qualify the figure instead of competing
    // with it — except the egress conflict, which is a warning and stays on the row, and
    // the tunnel, which is not a caveat on one figure but the reason every figure on the
    // row was measured a different way.
    expect(screen.getByText(/TLS · Filtering confirmed/)).toBeInTheDocument();
    expect(screen.getByText('Through a tunnel')).toBeVisible();
    expect(screen.getByRole('button', { name: 'What Through a tunnel means' })).toBeInTheDocument();
    expect(screen.getByText("Probe may not follow this app's route")).toBeInTheDocument();
    expect(screen.getByText("Probe may not follow this app's route")).toBeVisible();
    expect(
      screen.getByText(/This app's traffic leaves via Ethernet \(192\.0\.2\.10\)/),
    ).toBeInTheDocument();
  });

  it('shows three figures and no caveats until the reader asks for more', () => {
    // The whole item is about what is *not* shown by default, and that is exactly the kind
    // of thing that grows back one field at a time.
    render(
      <AppCard
        app={app({
          endpoints: [endpoint({ probeKind: 'tlsHello', tunnelled: true, recentBytes: 4096 })],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    for (const shown of ['Ping (RTT)', 'Jitter', 'Loss']) {
      expect(screen.getByText(shown)).toBeVisible();
    }
    // Present in the document — the expander is a disclosure, not a second request to Rust —
    // and not on screen until it is opened.
    for (const hidden of [
      'How it is being measured',
      'Which adapter it leaves by',
      'Data exchanged (30 s)',
      'In use',
    ]) {
      expect(screen.getByText(hidden)).not.toBeVisible();
    }
  });

  it('opens the caveats in place, with no setting to forget being in', async () => {
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ probeKind: 'tlsHello' })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByText('More about this connection'));

    expect(screen.getByText('How it is being measured')).toBeVisible();
    expect(screen.getByText('Data exchanged (30 s)')).toBeVisible();
  });

  it('says the first seconds are a warm-up rather than showing them as findings', () => {
    // The samples right after an application is picked are the least informative it will
    // ever have. Rust decides when that is over; the page states the time left rather than
    // showing three dashes that read like a failure.
    render(
      <AppCard
        app={app({
          warmupSecsRemaining: 42,
          endpoints: [endpoint({ warmupSecsRemaining: 42, jitterMs: null, lossPct: null })],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/Warming up · 42 s left/)).toBeInTheDocument();
    expect(screen.getByText('Warming up · 42 s')).toBeInTheDocument();
  });

  it('shows nothing about a warm-up once it is over', () => {
    render(
      <AppCard
        app={app()}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.queryByText(/Warming up/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Measuring this application/)).not.toBeInTheDocument();
  });

  it('gives every figure on the closed row an explanation reachable without a mouse', async () => {
    render(
      <AppCard
        app={app()}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    for (const metric of ['Ping (RTT)', 'Jitter', 'Loss']) {
      expect(screen.getByRole('button', { name: `What ${metric} means` })).toBeInTheDocument();
    }
    // Reached by keyboard, like everything else on this page. A real focus rather than a
    // synthetic event, because what is being asserted is that focus alone opens it.
    const help = screen.getByRole('button', { name: 'What Ping (RTT) means' });
    act(() => {
      help.focus();
    });
    expect(await screen.findByRole('note')).toBeInTheDocument();
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
        flowStatus="active"
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
        flowStatus="active"
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
        flowStatus="active"
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
    render(
      <AppCard
        app={app()}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getAllByText('Carrying traffic').length).toBeGreaterThan(0);
    expect(screen.queryByText('Unreachable')).not.toBeInTheDocument();
    // It claims nothing it did not measure.
    const rtt = screen.getByText('Ping (RTT)').parentElement?.parentElement;
    expect(rtt).toHaveTextContent('—');
    expect(
      screen.getByText('Data exchanged (30 s)').parentElement?.parentElement,
    ).toHaveTextContent('630 kB');
  });

  it('says how old a connection is, and which kind of old that is', () => {
    // What the user asked for: telling a new endpoint from one that has been there all
    // match. Two facts under two words — a TCP connection has an establishment the system
    // dates, a UDP endpoint has none — with the figure at level one and the word for it a
    // level down, so a reader can never mistake one claim for the other.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              transport: 'tcp',
              key: 'tcp/1.1.1.2:443',
              address: '1.1.1.2:443',
              // Degraded so the supporting-connections group is unfolded: whether it starts
              // folded is a separate rule with its own test, and this one is about the age.
              health: 'degraded',
              age: { secs: 5_400, kind: 'established' },
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/Age 1 h 30 min/)).toBeVisible();
    expect(screen.getByText('Connection established')).toBeInTheDocument();
    expect(screen.queryByText('Watched by this app')).not.toBeInTheDocument();
  });

  it('never borrows the word "established" for a UDP endpoint', () => {
    // There is no connection to have been established, so the only honest figure is how
    // long this application has been watched talking to the address.
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ age: { secs: 45, kind: 'watched' } })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/Age 45 s/)).toBeVisible();
    expect(screen.getByText('Watched by this app')).toBeInTheDocument();
    expect(screen.queryByText('Connection established')).not.toBeInTheDocument();
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    const stack = screen.getByText('Round trip measured by Windows').parentElement;
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.queryByText('Round trip measured by Windows')).not.toBeInTheDocument();
  });

  it('shows a dash rather than a zero where nothing counted the traffic', () => {
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ recentBytes: null, rttMs: null, lossPct: null })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    const traffic = screen.getByText('Data exchanged (30 s)').parentElement?.parentElement;
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText(/Nothing discovered yet/)).toBeInTheDocument();
  });

  it('lets the user stop following the application', async () => {
    // By application identity, never by process: the one the user picked may be long gone
    // while its children are still being measured.
    const onForget = vi.fn();
    render(
      <AppCard
        app={app({ id: 7 })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={onForget}
      />,
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
        flowStatus="active"
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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(lines()).toEqual([{ label: 'Route to 1.1.1.9:27015', isPath: true }]);
    // And the chart's own caption must not call any of it a ping: the chart draws dashed
    // routes as well as round trips, and a route belongs to a router short of the server.
    // It read "Ping over time" above exactly that.
    const caption = document.querySelector('.nm-appcard__chartnote');
    expect(caption?.textContent ?? '').not.toMatch(/ping/i);
    expect(caption?.textContent ?? '').toContain('One point per 3 s');
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
        flowStatus="active"
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
        flowStatus="active"
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
        flowStatus="active"
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

  it('brings a row chosen on the chart into view instead of highlighting it blind', async () => {
    // The complaint this answers: hovering a line highlighted a row that might be off
    // screen, so the only effect of touching the chart happened somewhere the reader was not
    // looking. On selection only — scrolling on hover is nauseating.
    const scrolled = vi.fn();
    Element.prototype.scrollIntoView = scrolled;

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
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    const [line] = screen
      .getAllByTestId('chart-line')
      .filter((node) => node.getAttribute('data-label') === '1.1.1.2:443');
    if (!line) throw new Error('the chart drew no line for that endpoint');
    await userEvent.click(line);

    expect(scrolled).toHaveBeenCalledWith({ block: 'nearest' });
    // And it is pinned, so what was scrolled to is also what is raised.
    const rows = screen
      .getAllByRole('listitem')
      .filter((row) => row.className.includes('nm-endpoint'));
    expect(rows.find((row) => row.id.endsWith('b'))?.className).toContain('nm-endpoint--raised');
  });

  it('lets a pinned endpoint be released again', async () => {
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ key: 'a', address: '1.1.1.1:27015' })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    const [chosen] = screen.getAllByRole('button', { name: /1\.1\.1\.1:27015/ });
    if (!chosen) throw new Error('the row offers no way to raise its endpoint');
    await userEvent.click(chosen);
    await userEvent.click(chosen);

    expect(chosen).toHaveAttribute('aria-pressed', 'false');
  });

  it('names the network an address belongs to, and keeps its number for the expander', async () => {
    // The name is the only thing on the row a player can recognise without knowing what a
    // single one of the figures means. The autonomous system number behind it is true and
    // searchable but means nothing to that reader, so it waits until they ask.
    render(
      <AppCard
        app={app({
          endpoints: [
            endpoint({
              network: { asn: 13335, name: 'CLOUDFLARENET', country: 'US' },
            }),
          ],
        })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.getByText('CLOUDFLARENET')).toBeVisible();
    expect(screen.getByText(/AS13335/)).not.toBeVisible();

    await userEvent.click(screen.getByText('More about this connection'));

    expect(screen.getByText('Whose network it is')).toBeVisible();
    expect(screen.getByText(/AS13335/)).toBeVisible();
    expect(screen.getByText(/registered in US/)).toBeVisible();
  });

  it('falls back to the number for a network the directory has no name for', () => {
    // Not friendly, but true and searchable — and better than dropping a fact the reader
    // could have used because half of it is missing.
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ network: { asn: 64500, name: null, country: null } })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    // Twice over: as the label at level one, standing in for the name it does not have, and
    // again in the expander where the number is the field's own subject.
    const [label] = screen.getAllByText('AS64500');
    expect(label).toBeVisible();
  });

  it('shows nothing where the network is unknown rather than guessing at one', async () => {
    // Absent stays absent. There is no nearest network to reach for, and a wrong name is not
    // a missing name — it is a false statement about where someone's traffic went.
    render(
      <AppCard
        app={app({ endpoints: [endpoint({ network: null })] })}
        trafficWindowSecs={30}
        chartStepSecs={3}
        flowStatus="active"
        onForget={vi.fn()}
      />,
    );

    expect(screen.queryByText(/AS\d/)).not.toBeInTheDocument();

    await userEvent.click(screen.getByText('More about this connection'));

    expect(screen.queryByText('Whose network it is')).not.toBeInTheDocument();
  });
});
