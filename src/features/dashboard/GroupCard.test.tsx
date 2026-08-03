import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { countChips, stateBadges } from '../../shared/testing';
import { GroupCard } from './GroupCard';
import type { GroupView, HealthView, TargetView } from '../../shared/ipc';

// uPlot draws to a canvas, which jsdom does not implement. The chart carries no
// information the row does not also state in text, so the tests replace it outright.
vi.mock('./Sparkline', () => ({
  Sparkline: ({ label }: { label: string }) => <div data-testid="sparkline" aria-label={label} />,
}));

const target = (overrides: Partial<TargetView> = {}): TargetView => ({
  key: 'foreign/a',
  label: 'Discord',
  writtenAddress: 'discord.com',
  resolvedAddress: '203.0.113.7:443',
  tunnelled: false,
  measurable: true,
  probeKind: 'icmpEcho',
  filteringConfirmed: false,
  health: 'ok',
  rttMs: 24,
  jitterMs: 3,
  lossPct: 0,
  seriesAgeSecs: [-2, -1, 0],
  seriesRttMs: [24, null, 25],
  ...overrides,
});

const group = (overrides: Partial<GroupView> = {}): GroupView => ({
  group: 'foreign',
  verdict: 'ok',
  counts: { ok: 1, degraded: 0, unreachable: 0, blocked: 0, carryingTraffic: 0, unknown: 0 },
  rttMs: 24,
  jitterMs: 3,
  lossPct: 0,
  targets: [target()],
  ...overrides,
});

describe('GroupCard', () => {
  it('names the baseline and its headline verdict', () => {
    render(<GroupCard group={group()} />);

    expect(screen.getByRole('heading', { name: 'Foreign' })).toBeInTheDocument();
    expect(
      screen.getByText('Services typically degraded or blocked at the border'),
    ).toBeInTheDocument();
    expect(screen.getAllByText('OK').length).toBeGreaterThan(0);
  });

  it('names its figures the way the rest of the app does, and explains each of them', () => {
    // The last figures on the merged Network page that could not explain themselves, sitting
    // directly above service cards that could. A group's are medians and say so — one member
    // on a bad path must not speak for the rest.
    render(<GroupCard group={group()} />);

    expect(screen.getByText('Ping, median')).toBeVisible();
    expect(screen.getByText('Jitter, median')).toBeVisible();
    expect(screen.getByText('Ping (RTT)')).toBeVisible();
    for (const figure of ['Ping, median', 'Jitter', 'Loss', 'Ping (RTT)']) {
      expect(
        screen.getAllByRole('button', { name: `What ${figure} means` }).length,
      ).toBeGreaterThan(0);
    }
  });

  it('shows the distribution rather than collapsing a mixed group to one colour', () => {
    // The requirement CLAUDE.md spells out: "3 clean, 1 unreachable" is actionable; a
    // single amber dot is not.
    render(
      <GroupCard
        group={group({
          verdict: 'degraded',
          counts: {
            ok: 3,
            degraded: 0,
            unreachable: 1,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 0,
          },
        })}
      />,
    );

    // Chips, not badges: "this baseline is unreachable" and "one of its targets is" are
    // different claims and must not be the same shape.
    expect(countChips(document)).toEqual(['1 Unreachable', '3 OK']);
  });

  it('omits states no member is in', () => {
    render(<GroupCard group={group()} />);
    expect(screen.queryByText(/Probe blocked/)).not.toBeInTheDocument();
  });

  it.each<[HealthView, string]>([
    ['ok', 'OK'],
    ['degraded', 'Degraded'],
    ['unreachable', 'Unreachable'],
    ['blocked', 'Probe blocked'],
    ['unknown', 'Not measured yet'],
  ])('renders the %s verdict as words, not only colour', (verdict, expected) => {
    render(<GroupCard group={group({ verdict, targets: [] })} />);
    // The badge specifically. A count chip carries the same words about a different subject,
    // which is why the two stopped looking alike.
    expect(stateBadges(document)).toContain(expected);
  });

  it('shows a dash rather than a zero where nothing was measured', () => {
    // A fabricated "0 ms / 0 % loss" would tell the user their network is fine before a
    // single probe has come back.
    render(
      <GroupCard
        group={group({
          verdict: 'unknown',
          rttMs: null,
          jitterMs: null,
          lossPct: null,
          counts: {
            ok: 0,
            degraded: 0,
            unreachable: 0,
            blocked: 0,
            carryingTraffic: 0,
            unknown: 1,
          },
          targets: [target({ health: 'unknown', rttMs: null, jitterMs: null, lossPct: null })],
        })}
      />,
    );

    expect(screen.queryByText('0')).not.toBeInTheDocument();
    expect(screen.getAllByText('—').length).toBe(6);
  });

  it('states why a filtered target has no loss figure', () => {
    render(
      <GroupCard
        group={group({
          targets: [
            target({
              health: 'blocked',
              lossPct: null,
              probeKind: 'tlsHello',
              filteringConfirmed: true,
            }),
          ],
        })}
      />,
    );

    const row = screen.getByText('Discord').closest('li');
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getByText('Probe blocked')).toBeInTheDocument();
    expect(within(row as HTMLElement).getByText('Filtering confirmed')).toBeInTheDocument();
    expect(within(row as HTMLElement).getByText('TLS')).toBeInTheDocument();
  });

  it('marks a tunnelled endpoint so its number is not read as a round trip to the server', () => {
    render(<GroupCard group={group({ targets: [target({ tunnelled: true })] })} />);
    expect(screen.getByText('Through a tunnel')).toBeInTheDocument();
  });

  it('keeps an unresolved target visible instead of shrinking the baseline', () => {
    render(
      <GroupCard
        group={group({
          targets: [target({ resolvedAddress: null, measurable: false, health: 'unknown' })],
        })}
      />,
    );

    expect(screen.getByText('Discord')).toBeInTheDocument();
    expect(screen.getByText('Name did not resolve')).toBeInTheDocument();
    // "Cannot be measured" would be a second, redundant claim about the same fact.
    expect(screen.queryByText('Cannot be measured')).not.toBeInTheDocument();
  });

  it('says when a resolved target has run out of probe kinds', () => {
    render(<GroupCard group={group({ targets: [target({ measurable: false })] })} />);
    expect(screen.getByText('Cannot be measured')).toBeInTheDocument();
  });

  it('reports an empty baseline instead of rendering nothing', () => {
    render(<GroupCard group={group({ targets: [], verdict: 'unknown' })} />);
    expect(screen.getByText('No targets in this baseline')).toBeInTheDocument();
  });
});
