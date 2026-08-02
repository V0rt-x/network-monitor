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
