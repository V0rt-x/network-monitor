import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import '../../i18n';
import type { PathView } from '../../shared/ipc';
import { PathPanel } from './PathPanel';

const path = (overrides: Partial<PathView> = {}): PathView => ({
  hopTtl: 12,
  hopsProbed: 3,
  position: 'beyondALongHaulLink',
  quality: 'ok',
  rttMs: 84.2,
  jitterMs: 2.5,
  lossPct: 0,
  ...overrides,
});

describe('PathPanel', () => {
  it('makes its claim in the figure’s own name rather than in a paragraph', () => {
    // The Phase 5 protection, after the page stopped carrying explanations: the number is
    // presented as a *router's*, and that is said by the label a reader cannot skip rather
    // than by prose underneath, which they can. The sentence itself is one keystroke away.
    render(<PathPanel path={path()} />);

    expect(screen.getByText('Round trip to that router')).toBeInTheDocument();
    expect(screen.getByText('The route towards it')).toBeInTheDocument();
    // No explanatory prose survives on the panel.
    expect(
      screen.queryByText(/deepest router that does answer on the way/),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/not the round trip to the server/i)).not.toBeInTheDocument();
    // And it is still explained, on demand, without a mouse.
    expect(
      screen.getByRole('button', { name: 'What The route towards it means' }),
    ).toBeInTheDocument();
  });

  it('never uses the word "ping" for a figure that is not one', () => {
    // Elsewhere the measured round trip is labelled "Ping (RTT)", because that is the word the
    // audience knows and there it is true. Here the server answered nothing at all, so how
    // much further it sits is unknown — calling this a ping would be the exact claim Phase 5
    // exists to prevent, and it would be made in the one word the reader trusts most.
    const { container } = render(<PathPanel path={path()} />);

    expect(container.textContent).not.toMatch(/ping/i);
  });

  it('states where the route stops', () => {
    render(<PathPanel path={path({ position: 'insideTheAccessNetwork' })} />);
    expect(screen.getByText(/stops inside your provider/)).toBeInTheDocument();
  });

  it('reports a figure confined to the last hop as ambiguous rather than as a bad path', () => {
    // Routers rate-limit echoes addressed to themselves while forwarding perfectly. Calling
    // that a degraded path would be a diagnosis the measurement does not support.
    render(<PathPanel path={path({ quality: 'uncorroborated' })} />);

    expect(screen.getByText(/may be that router, not the path/)).toBeInTheDocument();
    expect(screen.queryByText('Path degraded')).not.toBeInTheDocument();
  });

  it('shows a missing figure as absent rather than as zero', () => {
    render(
      <PathPanel
        path={path({ quality: 'lost', hopTtl: null, rttMs: null, jitterMs: null, lossPct: null })}
      />,
    );

    expect(screen.getByText('The route stopped answering')).toBeInTheDocument();
    expect(screen.queryByText('0 ms')).not.toBeInTheDocument();
    expect(screen.queryByText('0 %')).not.toBeInTheDocument();
  });
});
