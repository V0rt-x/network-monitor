import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import '../../i18n';
import { HelpPage } from './HelpPage';
import { anchorOf, HELP_SECTIONS, HELP_TOPICS } from './topics';

describe('HelpPage', () => {
  it('explains every topic a label can point at', () => {
    // The list is shared with the labels, so a topic can never have a disclosure with no
    // section behind it — but a missing translation would still render a bare key, and that
    // is what this catches.
    render(<HelpPage topic={null} onBack={null} />);

    for (const topic of HELP_TOPICS) {
      const section = document.getElementById(anchorOf(topic));
      expect(section).not.toBeNull();
      expect(section?.textContent ?? '').not.toContain('help.topic');
      expect((section?.textContent ?? '').length).toBeGreaterThan(80);
    }
  });

  it('opens as an index: every title, every one-line summary, and no bodies', () => {
    // Twenty-eight topics each opening with a full body is about six thousand words with no
    // point of entry, and a reader who followed one "Learn more" then had to find their
    // answer inside it.
    render(<HelpPage topic={null} onBack={null} />);

    expect(screen.getAllByRole('heading', { level: 4 })).toHaveLength(HELP_TOPICS.length);
    expect(document.querySelectorAll('.nm-help-page__short')).toHaveLength(HELP_TOPICS.length);
    expect(document.querySelectorAll('.nm-help-page__body')).toHaveLength(0);
  });

  it('shows exactly the body that was asked for', async () => {
    render(<HelpPage topic={null} onBack={null} />);

    const jitter = document.getElementById(anchorOf('jitter'));
    const readMore = jitter?.querySelector('button');
    if (!readMore) throw new Error('the jitter topic has no "Read more"');
    await userEvent.click(readMore);

    const opened = jitter?.querySelectorAll('.nm-help-page__body').length ?? 0;
    expect(opened).toBeGreaterThan(0);
    // And nothing else on the page opened with it.
    expect(document.querySelectorAll('.nm-help-page__body')).toHaveLength(opened);
  });

  it('keeps the paragraph breaks its bodies were written with', async () => {
    // Inside a single `<p>` a blank line collapses to a space, which shipped the longest
    // topic as one unbroken ~350-word block.
    render(<HelpPage topic="network" onBack={null} />);

    await Promise.resolve();
    const section = document.getElementById(anchorOf('network'));
    expect(section?.querySelectorAll('.nm-help-page__body').length).toBeGreaterThan(1);
  });

  it('groups the topics and offers a contents to reach them by', () => {
    render(<HelpPage topic={null} onBack={null} />);

    const contents = screen.getByRole('navigation', { name: 'Contents' });
    expect(contents.querySelectorAll('a')).toHaveLength(HELP_SECTIONS.length);
    expect(screen.getByRole('heading', { name: 'Verdicts' })).toBeInTheDocument();
  });

  it('leads with why this is not the ping the game shows', () => {
    // The single most important string in the application: without it the honest answer
    // looks like a wrong one.
    render(<HelpPage topic={null} onBack={null} />);

    const headings = screen.getAllByRole('heading', { level: 4 });
    expect(headings[0]).toHaveTextContent('Why this is not the ping your game shows');
  });

  it('narrows to the topics whose words the reader typed', async () => {
    render(<HelpPage topic={null} onBack={null} />);

    await userEvent.type(screen.getByLabelText('Filter topics'), 'freeze');

    const headings = screen.getAllByRole('heading', { level: 4 }).map((node) => node.textContent);
    expect(headings).toEqual(['Freeze']);
  });

  it('says a filter matched nothing rather than looking broken', async () => {
    render(<HelpPage topic={null} onBack={null} />);

    await userEvent.type(screen.getByLabelText('Filter topics'), 'zzzzz');

    expect(screen.getByText('No topic matches that.')).toBeInTheDocument();
    expect(screen.queryAllByRole('heading', { level: 4 })).toHaveLength(0);
  });

  it('puts a reader who followed a "Learn more" back where they came from', async () => {
    const back = vi.fn();
    render(<HelpPage topic="jitter" onBack={back} />);

    await userEvent.click(screen.getByRole('button', { name: 'Back' }));

    expect(back).toHaveBeenCalled();
  });

  it('offers no way back to a reader who arrived by the tab', () => {
    // There is nowhere they were taken away from, and a Back that guessed would be worse
    // than none.
    render(<HelpPage topic={null} onBack={null} />);

    expect(screen.queryByRole('button', { name: 'Back' })).not.toBeInTheDocument();
  });

  it('marks the topic the reader was sent to, and opens it', () => {
    render(<HelpPage topic="freeze" onBack={null} />);

    expect(document.getElementById(anchorOf('freeze'))?.className).toContain('--opened');
    expect(document.getElementById(anchorOf('loss'))?.className).not.toContain('--opened');
    expect(
      document.getElementById(anchorOf('freeze'))?.querySelectorAll('.nm-help-page__body').length,
    ).toBeGreaterThan(0);
  });

  it('promises nothing it would have to fetch', () => {
    // Bundled, not a website: an external link is a request this product promised never to
    // make on the user's behalf, and it is useless to someone being filtered.
    render(<HelpPage topic={null} onBack={null} />);

    for (const link of screen.getAllByRole('link')) {
      expect(link.getAttribute('href')).toMatch(/^#/);
    }
  });
});
