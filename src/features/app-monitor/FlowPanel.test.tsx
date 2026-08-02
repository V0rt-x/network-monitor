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
  it('says none of these is a ping by never naming one, not by saying so', () => {
    render(<FlowPanel flow={flow()} />);

    // The rule the whole phase protects: this column and the route column are two
    // quantities, and neither may be presented as a round trip to the server. On a page that
    // carries no explanations, what makes that point is that not one label here is a round
    // trip — a name cannot be skipped, and the sentence is on the heading's ⓘ for whoever
    // wants it.
    expect(screen.queryByText(/Nothing here is a ping/)).not.toBeInTheDocument();
    expect(screen.queryByText(/without sending anything/)).not.toBeInTheDocument();
    expect(screen.queryByText(/round trip/i)).not.toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'What The traffic itself means' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Updates from the server')).toBeInTheDocument();
    expect(screen.getByText('20.0 a second')).toBeInTheDocument();
  });

  it('qualifies its jitter so it can never be read as the probe’s', () => {
    // All five figures stay. The one with a standard network term behind it carries that
    // term — but *arrival* jitter, because the probe's own jitter can be read on the same
    // card and two figures called "jitter" would be worse than an invented word. The rest
    // name quantities with no standard term to return to, so they stay experiential.
    render(<FlowPanel flow={flow({ receiveShortfallPct: 12 })} />);

    expect(screen.getByText('Arrival jitter')).toBeInTheDocument();
    expect(screen.queryByText('Jitter')).not.toBeInTheDocument();
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
