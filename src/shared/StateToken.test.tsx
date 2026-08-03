import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import '../i18n';
import { Distribution } from '../features/app-monitor/Distribution';
import type { HealthCountsView, HealthView } from './ipc';
import { StateToken } from './StateToken';

const HEALTHS: readonly HealthView[] = [
  'ok',
  'degraded',
  'unreachable',
  'blocked',
  'carryingTraffic',
  'unknown',
];

/** What every state is called, in the order above. */
const WORDS = [
  'OK',
  'Degraded',
  'Unreachable',
  'Probe blocked',
  'Carrying traffic',
  'Not measured yet',
];

describe('StateToken', () => {
  it('carries its state’s word as its accessible name, in every state there is', () => {
    // The word is never gone — it is what a screen reader gets at all times, whatever the
    // reader does or does not hover. A token that only had a colour would be the one place
    // in this product where colour is the sole carrier of meaning.
    for (const [index, health] of HEALTHS.entries()) {
      const { unmount } = render(<StateToken health={health} />);
      expect(screen.getByRole('img')).toHaveAccessibleName(WORDS[index] ?? '');
      unmount();
    }
  });

  it('differs in shape as well as in colour', () => {
    // Twelve hues are distinguishable to most people and to nobody with a red-green
    // deficiency. Six shapes crossed with six colours are.
    const shapes = new Set<string>();
    for (const health of HEALTHS) {
      const { container, unmount } = render(<StateToken health={health} />);
      const mark = container.querySelector('.nm-token');
      shapes.add(mark?.className ?? '');
      // The colour is a class of its own, so a chip and a pill can share it without sharing
      // a shape.
      expect(mark).toHaveClass(`nm-tone--${health}`);
      unmount();
    }
    expect(shapes.size).toBe(HEALTHS.length);
  });

  it('names its qualifiers too, and spells them out on hover', async () => {
    render(
      <StateToken
        health="ok"
        qualifiers={[
          { kind: 'warmup', name: 'Warming up · 46 s' },
          { kind: 'tunnelled', name: 'Through a tunnel' },
        ]}
      />,
    );

    const token = screen.getByRole('img');
    expect(token).toHaveAccessibleName('OK · Warming up · 46 s · Through a tunnel');
    expect(screen.queryByText('OK · Warming up · 46 s · Through a tunnel')).toBeNull();

    await userEvent.hover(token);

    expect(screen.getByText('OK · Warming up · 46 s · Through a tunnel')).toBeVisible();
  });

  it('spells them out on focus, so a keyboard reaches what a pointer does', async () => {
    render(<StateToken health="degraded" />);

    await userEvent.tab();

    expect(screen.getByRole('img')).toHaveFocus();
    expect(screen.getByText('Degraded')).toBeVisible();
  });

  it('is one focusable group, however many marks it holds', () => {
    // Three focusable marks a row would be the mistake 6.7 undid when it cut two hundred and
    // sixty ⓘ marks to nine.
    const { container } = render(
      <StateToken
        health="ok"
        qualifiers={[
          { kind: 'warmup', name: 'Warming up · 46 s' },
          { kind: 'tunnelled', name: 'Through a tunnel' },
        ]}
      />,
    );

    expect(container.querySelectorAll('[tabindex]')).toHaveLength(1);
    expect(container.querySelectorAll('.nm-token')).toHaveLength(3);
  });
});

describe('what keeps the page honest with the words gone from the row', () => {
  it('still names every state in words in the distribution', () => {
    // This is the whole reason a state can afford to be a mark: the group heading above the
    // rows says "4 clean, 2 degraded, 1 unreachable" in words at level one, and Rust's
    // worst-first order still puts the bad rows at the top. The token is a second channel
    // for a distribution that is already stated, not the only place the state exists.
    const counts: HealthCountsView = {
      ok: 4,
      degraded: 2,
      unreachable: 1,
      blocked: 0,
      carryingTraffic: 0,
      unknown: 0,
    };
    render(<Distribution counts={counts} label="Endpoint states" />);

    const list = screen.getByRole('list', { name: 'Endpoint states' });
    expect(list.textContent).toContain('Unreachable');
    expect(list.textContent).toContain('Degraded');
    expect(list.textContent).toContain('OK');
  });
});
