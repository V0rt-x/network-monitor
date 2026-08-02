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
    expect(screen.getByText(/Nothing here is a ping/)).toBeInTheDocument();
    expect(screen.getByText('Updates from the server')).toBeInTheDocument();
    expect(screen.getByText('20.0 a second')).toBeInTheDocument();
  });

  it('names each figure for what the player experiences', () => {
    // Reworded, not thinned: all five stay, and each is named for the thing a player feels
    // rather than for the quantity an engineer would name.
    render(<FlowPanel flow={flow({ receiveShortfallPct: 12 })} />);

    expect(screen.getByText('Evenness')).toBeInTheDocument();
    expect(screen.getByText('Worst pause')).toBeInTheDocument();
    expect(screen.getByText('Drop-off')).toBeInTheDocument();
    expect(screen.getByText('119')).toBeInTheDocument();
    expect(screen.getByText('1.4')).toBeInTheDocument();
  });

  it('keeps a freeze prominent rather than demoting it with the details', () => {
    // Only the far end knows what it sent, so a datagram that never arrived is invisible
    // here. What can honestly be said is that nothing has come back while we kept sending —
    // and it is the one figure a player recognises immediately.
    render(<FlowPanel flow={flow({ stallMs: 1_400 })} />);

    expect(screen.getByText(/Frozen for 1400 ms/)).toBeInTheDocument();
    expect(screen.queryByText(/lost|loss/i)).not.toBeInTheDocument();
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

    expect(screen.getByText(/Not enough arriving yet/)).toBeInTheDocument();
    expect(screen.queryByText('0')).not.toBeInTheDocument();
    expect(screen.queryByText('0 B/s')).not.toBeInTheDocument();
  });

  it('names the drop-off carefully, never as packet loss', () => {
    // Only the far end knows what it sent, so a datagram that never arrived is invisible
    // from here. The name keeps that distinction in every language.
    render(<FlowPanel flow={flow({ receiveShortfallPct: 46 })} />);

    expect(screen.getByText('Drop-off')).toBeInTheDocument();
    expect(screen.getByText('46')).toBeInTheDocument();
    expect(screen.queryByText(/packet loss/i)).not.toBeInTheDocument();
  });
});
