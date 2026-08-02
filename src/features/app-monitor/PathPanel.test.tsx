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
  it('leads with one number, and never calls it the server', () => {
    render(<PathPanel path={path()} />);

    // The whole point: the number is presented as a router's, with the gap stated. The hop
    // count and where the route stops are a level down, in the row's own expander — they
    // qualify the figure rather than being what a player reads.
    expect(screen.getByText('Round trip to that router')).toBeInTheDocument();
    expect(screen.getByText(/deepest router that does answer on the way/)).toBeInTheDocument();
    expect(screen.getByText(/not the round trip to the server/i)).toBeInTheDocument();
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
