import { render, screen, within } from '@testing-library/react';
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
