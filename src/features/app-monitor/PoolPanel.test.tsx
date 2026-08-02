import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
    // know that before reading a verdict that stops at the route — so the panel stays.
    // What that costs them is a sentence in the help, not a paragraph on the card.
    render(<PoolPanel pool={null} />);

    expect(screen.getByText('None for this application.')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: "What The game's own reference servers means" }),
    ).toBeInTheDocument();
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

    // The count stays on the card because it changes what the percentage above it means;
    // *why* a silent member proves nothing is the ⓘ's answer rather than a paragraph's.
    expect(screen.getByText('4 not counted — never answered a probe')).toBeInTheDocument();
  });

  it('says nothing about unproven members when there are none', () => {
    render(<PoolPanel pool={pool()} />);
    expect(screen.queryByText(/never answered a probe/u)).not.toBeInTheDocument();
  });

  it('explains what the pool is evidence for when asked, not before', async () => {
    // The paragraph saying what the pool proves was competing with the three figures it
    // qualifies. It is a real explanation and it is not a warning, so it went a level down.
    render(<PoolPanel pool={pool()} />);

    expect(
      screen.queryByText(/points at the game rather than at your path/u),
    ).not.toBeInTheDocument();

    await userEvent.click(
      screen.getByRole('button', { name: "What The game's own reference servers means" }),
    );
    expect(await screen.findByRole('note')).toHaveTextContent(
      /All of them silent while your baselines are clean/,
    );
  });
});
