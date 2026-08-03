import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { HelpProvider } from './HelpProvider';
import { MetricHelp } from './MetricHelp';

describe('MetricHelp', () => {
  it('says nothing until it is asked', () => {
    render(<MetricHelp topic="jitter" />);

    expect(screen.queryByRole('note')).not.toBeInTheDocument();
    expect(screen.getByRole('button')).toHaveAttribute('aria-expanded', 'false');
  });

  it('is the label itself, with no second mark beside it', () => {
    // The rule this implements was written in the singular — "an ⓘ on every figure" — and
    // silently assumed one figure on screen. At twenty connections it produced up to two
    // hundred and sixty identical marks, and a mark repeated two hundred times does not
    // explain a figure, it hides it. The word already names the quantity; a dotted underline
    // says there is more to it, costs no width, and adds no second target for a keyboard.
    render(<MetricHelp topic="jitter">Jitter</MetricHelp>);

    const label = screen.getByRole('button');
    expect(label).toHaveTextContent('Jitter');
    expect(label).toHaveClass('nm-explains');
    expect(document.querySelectorAll('.nm-help__mark')).toHaveLength(0);
  });

  it('falls back to the topic’s own title when the label is the same words', () => {
    // A column heading reading "Ping (RTT)" is explained by a topic whose title is the same
    // words; repeating them at the call site is a second place to keep them in step.
    render(<MetricHelp topic="rtt" />);

    expect(screen.getByRole('button')).toHaveTextContent('Ping (RTT)');
  });

  it('explains the metric in place, without a mouse', async () => {
    // The whole point of the level: a player who does not know what jitter is has to be able
    // to ask, and a keyboard user has to be able to ask the same way.
    render(<MetricHelp topic="jitter" />);

    await userEvent.tab();
    expect(await screen.findByRole('note')).toHaveTextContent(/varies between probes/i);
  });

  it('opens the bundled help at its own section', async () => {
    const openHelp = vi.fn();
    render(
      <HelpProvider openHelp={openHelp}>
        <MetricHelp topic="dropOff" />
      </HelpProvider>,
    );

    await userEvent.click(screen.getByRole('button', { name: /what drop-off means/i }));
    await userEvent.click(screen.getByRole('button', { name: 'Learn more' }));

    expect(openHelp).toHaveBeenCalledWith('dropOff');
  });

  it('flips off both edges of the window, not just the right one', async () => {
    // 6.7 measured the horizontal overflow and left the vertical one, so a panel opened on
    // the last row of a long table hung below the window — where the sentence explaining a
    // figure is exactly as unreadable as it is past the right edge. jsdom lays nothing out,
    // so the rectangle is the thing under test and it is supplied.
    const rect = vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue({
      width: 352,
      height: 120,
      top: 700,
      bottom: window.innerHeight + 120,
      left: window.innerWidth - 40,
      right: window.innerWidth + 312,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });

    render(<MetricHelp topic="jitter" />);
    await userEvent.tab();

    expect(await screen.findByRole('note')).toHaveClass(
      'nm-help__panel',
      'nm-help__panel--flipped',
      'nm-help__panel--above',
    );
    rect.mockRestore();
  });

  it('flips nowhere when the panel is taller than the room above it', async () => {
    // Flipping upwards a panel with nowhere to go trades one clipped sentence for another.
    const rect = vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue({
      width: 352,
      height: 900,
      top: 40,
      bottom: window.innerHeight + 400,
      left: 0,
      right: 352,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });

    render(<MetricHelp topic="jitter" />);
    await userEvent.tab();

    const note = await screen.findByRole('note');
    expect(note).not.toHaveClass('nm-help__panel--above');
    expect(note).not.toHaveClass('nm-help__panel--flipped');
    rect.mockRestore();
  });

  it('does not throw when rendered outside the shell', async () => {
    // A missing provider must not be an exception in the middle of a measurement someone is
    // reading. It simply does nothing.
    render(<MetricHelp topic="loss" />);

    await userEvent.click(screen.getByRole('button', { name: /what loss means/i }));
    await userEvent.click(screen.getByRole('button', { name: 'Learn more' }));

    // Still standing, still explaining itself: the door simply led nowhere.
    expect(screen.getByRole('note')).toBeInTheDocument();
  });
});
