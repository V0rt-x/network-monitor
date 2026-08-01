import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import '../../i18n';
import type { FlowView } from '../../shared/ipc';
import { FlowPanel } from './FlowPanel';

const flow = (overrides: Partial<FlowView> = {}): FlowView => ({
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
  ...overrides,
});

describe('FlowPanel', () => {
  it('states what the figures are measured from, and that none of them is a ping', () => {
    render(<FlowPanel flow={flow()} />);

    expect(screen.getByText(/without sending anything/)).toBeInTheDocument();
    // The rule the whole phase protects: this column and the route column are two
    // quantities, and neither may be presented as a round trip to the server.
    expect(screen.getByText(/Not a round-trip time/)).toBeInTheDocument();
    expect(screen.getByText(/20.0 updates a second/)).toBeInTheDocument();
    expect(screen.getByText(/over the last 10 s/)).toBeInTheDocument();
  });

  it('shows the worst gap beside the spread, because that is the hitch a player felt', () => {
    render(<FlowPanel flow={flow()} />);

    expect(screen.getByText('Worst gap (ms)')).toBeInTheDocument();
    expect(screen.getByText('119')).toBeInTheDocument();
    expect(screen.getByText('1.4')).toBeInTheDocument();
  });

  it('calls a one-way outage a stall rather than a loss figure', () => {
    // Only the far end knows what it sent, so a datagram that never arrived is invisible
    // here. What can honestly be said is that nothing has come back while we kept sending.
    render(<FlowPanel flow={flow({ stallMs: 1_400 })} />);

    expect(screen.getByText('Nothing back for 1400 ms')).toBeInTheDocument();
    expect(screen.queryByText(/loss/i)).not.toBeInTheDocument();
  });

  it('shows an absent figure as absent rather than as zero', () => {
    render(
      <FlowPanel
        flow={flow({
          updatesPerSec: null,
          arrivalJitterMs: null,
          arrivalMaxMs: null,
          receivedBytesPerSec: null,
        })}
      />,
    );

    expect(screen.getByText(/Not enough arrivals yet/)).toBeInTheDocument();
    expect(screen.queryByText('0')).not.toBeInTheDocument();
    expect(screen.queryByText('0 B/s')).not.toBeInTheDocument();
  });

  it('names the shortfall as a shortfall, never as packet loss', () => {
    render(<FlowPanel flow={flow({ receiveShortfallPct: 46 })} />);

    expect(screen.getByText('Return shortfall (%)')).toBeInTheDocument();
    expect(screen.getByText('46')).toBeInTheDocument();
  });
});
