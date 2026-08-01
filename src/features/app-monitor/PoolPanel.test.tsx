import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import '../../i18n';
import type { PoolView } from '../../shared/ipc';
import { PoolPanel } from './PoolPanel';

const pool = (overrides: Partial<PoolView> = {}): PoolView => ({
  seeded: 8,
  learned: 3,
  unproven: 0,
  health: 'ok',
  counts: { ok: 11, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  answeringPct: 100,
  rttMs: 42,
  ...overrides,
});

describe('PoolPanel', () => {
  it('says an application has no reference servers rather than hiding the panel', () => {
    // An absent pool can neither report an outage nor rule one out, and the user has to
    // know that before reading a verdict that stops at the route.
    render(<PoolPanel pool={null} />);
    expect(screen.getByText(/No reference servers for this application yet/u)).toBeInTheDocument();
  });

  it('shows the share answering and where the members came from', () => {
    render(<PoolPanel pool={pool()} />);

    expect(screen.getByText('100')).toBeInTheDocument();
    expect(screen.getByText('8 published, 3 learned')).toBeInTheDocument();
  });

  it('shows a dash rather than zero when nothing could be judged', () => {
    // Every probe filtered is an absence of knowledge; a "0 %" answering figure would be a
    // finding the pool never earned.
    render(
      <PoolPanel
        pool={pool({
          health: 'blocked',
          answeringPct: null,
          rttMs: null,
          counts: {
            ok: 0,
            degraded: 0,
            unreachable: 0,
            blocked: 11,
            carryingTraffic: 0,
            unknown: 0,
          },
        })}
      />,
    );

    expect(screen.getAllByText('—')).toHaveLength(2);
    expect(screen.getByText('Probe blocked')).toBeInTheDocument();
  });

  it('reports a partial outage as a partial one', () => {
    render(<PoolPanel pool={pool({ health: 'degraded', answeringPct: 50 })} />);

    expect(screen.getByText('Degraded')).toBeInTheDocument();
    expect(screen.getByText('50')).toBeInTheDocument();
  });

  it('says which members it cannot speak for', () => {
    // A learned member is usually a match server that answers nothing anyone can send.
    // Its silence proves nothing, and the panel has to say so rather than let a small
    // "answering" figure imply an outage.
    render(
      <PoolPanel
        pool={pool({
          unproven: 4,
          counts: {
            ok: 8,
            degraded: 0,
            unreachable: 0,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 0,
          },
        })}
      />,
    );

    expect(screen.getByText(/4 members have never answered a probe/u)).toBeInTheDocument();
  });

  it('says nothing about unproven members when there are none', () => {
    render(<PoolPanel pool={pool()} />);
    expect(screen.queryByText(/never answered a probe/u)).not.toBeInTheDocument();
  });

  it('explains what the pool is evidence for', () => {
    render(<PoolPanel pool={pool()} />);
    expect(screen.getByText(/points at the game rather than at your path/u)).toBeInTheDocument();
  });
});
