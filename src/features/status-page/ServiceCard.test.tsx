import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import '../../i18n';
import { ServiceCard } from './ServiceCard';
import type { CheckView, ServiceEndpointView, ServiceView } from '../../shared/ipc';

const checks = (marks: CheckView['mark'][]): CheckView[] =>
  marks.map((mark, index) => ({ ageSecs: -(marks.length - index - 1) * 45, mark }));

const endpoint = (overrides: Partial<ServiceEndpointView> = {}): ServiceEndpointView => ({
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
  checks: checks(['answered', 'answered', 'answered']),
  ...overrides,
});

const service = (overrides: Partial<ServiceView> = {}): ServiceView => ({
  id: 'steam',
  label: 'Steam',
  group: 'gamingPlatform',
  verdict: 'ok',
  counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  rttMs: 42,
  lossPct: 0,
  lastCheckedSecs: 12,
  endpoints: [endpoint()],
  ...overrides,
});

describe('ServiceCard', () => {
  it('names the service, its verdict and when it was last checked', () => {
    render(<ServiceCard service={service()} checkIntervalSecs={45} />);

    expect(screen.getByRole('heading', { name: 'Steam' })).toBeInTheDocument();
    expect(screen.getByText('OK')).toBeInTheDocument();
    expect(screen.getByText('Checked 12 s ago')).toBeInTheDocument();
  });

  it('says when nothing has been checked yet rather than showing a zero', () => {
    render(
      <ServiceCard
        service={service({ verdict: 'unknown', rttMs: null, lastCheckedSecs: null })}
        checkIntervalSecs={45}
      />,
    );

    expect(screen.getByText('Not checked yet')).toBeInTheDocument();
    // The dash, never a fabricated latency.
    expect(screen.getByText('—')).toBeInTheDocument();
  });

  it('warns when the checks themselves have stopped arriving', () => {
    // A status page whose data quietly stopped looks exactly like one reporting calm, so
    // the age has to become a warning rather than just a larger number.
    const { container } = render(
      <ServiceCard service={service({ lastCheckedSecs: 200 })} checkIntervalSecs={45} />,
    );

    expect(container.querySelector('.nm-service__stale')).not.toBeNull();
    expect(container.querySelector('.nm-service__checked')).toBeNull();
  });

  it('does not call a fresh check stale', () => {
    const { container } = render(
      <ServiceCard service={service({ lastCheckedSecs: 44 })} checkIntervalSecs={45} />,
    );

    expect(container.querySelector('.nm-service__stale')).toBeNull();
  });

  it('shows the distribution when a service has endpoints that disagree', () => {
    // A storefront answering while the gateway does not is the finding; one amber dot
    // would hide which half is broken.
    render(
      <ServiceCard
        service={service({
          verdict: 'degraded',
          counts: {
            ok: 1,
            degraded: 0,
            unreachable: 1,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 0,
          },
          endpoints: [
            endpoint(),
            endpoint({
              key: 'steam/store',
              writtenAddress: 'store.steampowered.com',
              health: 'unreachable',
              rttMs: null,
              meanRttMs: null,
              checks: checks(['lost', 'lost']),
            }),
          ],
        })}
        checkIntervalSecs={45}
      />,
    );

    expect(screen.getByText('1 OK')).toBeInTheDocument();
    expect(screen.getByText('1 Unreachable')).toBeInTheDocument();
    expect(screen.getByText('store.steampowered.com')).toBeInTheDocument();
  });

  it('does not repeat the headline on the only endpoint underneath it', () => {
    render(<ServiceCard service={service()} checkIntervalSecs={45} />);
    expect(screen.getAllByText('OK')).toHaveLength(1);
  });

  it('draws one timeline cell per check, each naming what it produced', () => {
    render(
      <ServiceCard
        service={service({
          endpoints: [endpoint({ checks: checks(['answered', 'lost', 'slow', 'filtered']) })],
        })}
        checkIntervalSecs={45}
      />,
    );

    const strip = screen.getByRole('list', {
      name: 'Recent checks of Steam at api.steampowered.com',
    });
    const cells = within(strip).getAllByRole('listitem');
    expect(cells).toHaveLength(4);
    // Colour is never the only channel: every cell carries its word.
    expect(within(strip).getByText('No answer')).toBeInTheDocument();
    expect(within(strip).getByText('Answered slowly')).toBeInTheDocument();
    expect(within(strip).getByText('Probe filtered')).toBeInTheDocument();
  });

  it('says what each figure is, rather than leaving them to be guessed', () => {
    // The complaint this answers: the card showed "Latest", "Mean", "Loss" and a bare
    // number, with nothing saying what a check was or which round trip the headline meant.
    // "Latest" in particular invites the reader to average the strip beside it by eye,
    // which is exactly the wrong reading. The span they cover is stated once on the page,
    // in the legend, rather than on both figures of every card.
    render(<ServiceCard service={service()} checkIntervalSecs={45} />);

    expect(screen.getByText('Ping, median')).toBeVisible();
    expect(screen.getByText('Ping, last check')).toBeVisible();
    expect(screen.getByText('Ping, mean')).toBeVisible();
    expect(screen.getByText('Loss')).toBeVisible();
  });

  it('says nothing under a strip nobody is pointing at', () => {
    // Twenty-three copies of "point at a cell to read when it happened" was the largest
    // block of prose in the product, and it explained rather than reported. The reading
    // appears when it is asked for; how to ask is on the legend's ⓘ, once.
    render(<ServiceCard service={service()} checkIntervalSecs={45} />);

    expect(screen.getByRole('status')).toHaveTextContent('');
    expect(screen.queryByText(/move through the strip/)).not.toBeInTheDocument();
  });

  it('gives every figure on the card an explanation reachable without a mouse', () => {
    render(<ServiceCard service={service()} checkIntervalSecs={45} />);

    for (const figure of ['Ping, median', 'Ping, last check', 'Ping, mean', 'Loss']) {
      expect(screen.getByRole('button', { name: `What ${figure} means` })).toBeInTheDocument();
    }
  });

  it('reads out a cell that is pointed at, with when it happened', async () => {
    render(
      <ServiceCard
        service={service({
          endpoints: [endpoint({ checks: checks(['answered', 'lost', 'answered']) })],
        })}
        checkIntervalSecs={45}
      />,
    );

    const strip = screen.getByRole('list', {
      name: 'Recent checks of Steam at api.steampowered.com',
    });
    const [, middle] = within(strip).getAllByRole('listitem');
    if (middle === undefined) throw new Error('the strip should have drawn three cells');
    await userEvent.hover(middle);

    // The middle cell of three, 45 s apart: one check back from the newest.
    expect(screen.getByRole('status')).toHaveTextContent('Check 2 of 3 · No answer · 45 s ago');
  });

  it('lets the arrow keys walk the strip, with one tab stop for the whole of it', async () => {
    // A tab stop per cell would put several hundred of them on a page of thirteen services,
    // between a keyboard user and the next thing they wanted. The reading has to be
    // reachable all the same — a title attribute is not.
    render(
      <ServiceCard
        service={service({
          endpoints: [endpoint({ checks: checks(['lost', 'slow', 'answered']) })],
        })}
        checkIntervalSecs={45}
      />,
    );

    const strip = screen.getByRole('list', {
      name: 'Recent checks of Steam at api.steampowered.com',
    });
    strip.focus();
    expect(strip).toHaveFocus();

    // Landing on the newest check, then stepping back through the older ones.
    await userEvent.keyboard('{ArrowLeft}');
    expect(screen.getByRole('status')).toHaveTextContent('Check 2 of 3 · Answered slowly');
    await userEvent.keyboard('{Home}');
    expect(screen.getByRole('status')).toHaveTextContent('Check 1 of 3 · No answer · 90 s ago');
    // And it cannot be walked off the end.
    await userEvent.keyboard('{ArrowLeft}');
    expect(screen.getByRole('status')).toHaveTextContent('Check 1 of 3');
  });

  it('says a service has not been checked yet instead of drawing an empty strip', () => {
    render(
      <ServiceCard
        service={service({ endpoints: [endpoint({ checks: [] })] })}
        checkIntervalSecs={45}
      />,
    );

    expect(screen.getByText('Not checked yet')).toBeInTheDocument();
  });

  it('discloses a tunnelled endpoint rather than presenting its figure as a round trip', () => {
    render(
      <ServiceCard
        service={service({ endpoints: [endpoint({ tunnelled: true })] })}
        checkIntervalSecs={45}
      />,
    );

    expect(screen.getByText('Through a tunnel')).toBeInTheDocument();
    // The only badge with an ⓘ, because it is the reason the figures beside it were
    // measured a different way rather than a caveat on one of them.
    expect(screen.getByRole('button', { name: 'What Through a tunnel means' })).toBeInTheDocument();
  });

  it('names a check the tunnel answered rather than calling it a lost packet', () => {
    // Before this state existed, a mark the view had no word for fell through to "no
    // answer" — reporting a dropped packet where a packet was never sent. It is also not
    // "probe filtered": filtering happens on the path, and this happened before it.
    render(
      <ServiceCard
        service={service({
          endpoints: [endpoint({ checks: checks(['answeredLocally', 'answered']) })],
        })}
        checkIntervalSecs={45}
      />,
    );

    const strip = screen.getByRole('list', {
      name: 'Recent checks of Steam at api.steampowered.com',
    });
    expect(within(strip).getByText('Your tunnel answered')).toBeInTheDocument();
    expect(within(strip).queryByText('No answer')).not.toBeInTheDocument();
    expect(within(strip).queryByText('Probe filtered')).not.toBeInTheDocument();
  });

  it('marks an endpoint whose name never resolved', () => {
    render(
      <ServiceCard
        service={service({ endpoints: [endpoint({ resolvedAddress: null, measurable: false })] })}
        checkIntervalSecs={45}
      />,
    );

    expect(screen.getByText('Name did not resolve')).toBeInTheDocument();
  });

  it('shows a dash for a loss figure that was never measured', () => {
    render(
      <ServiceCard
        service={service({ endpoints: [endpoint({ lossPct: null, rttMs: null })] })}
        checkIntervalSecs={45}
      />,
    );

    expect(screen.getAllByText('—').length).toBeGreaterThan(0);
  });
});
