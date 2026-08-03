import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import '../../i18n';
import { HelpPage } from './HelpPage';
import { anchorOf, HELP_TOPICS } from './topics';

describe('HelpPage', () => {
  it('explains every topic a metric can point at', () => {
    // The list is shared with the ⓘ, so a topic can never have a tooltip with no section
    // behind it — but a missing translation would still render a bare key, and that is what
    // this catches.
    render(<HelpPage topic={null} />);

    for (const topic of HELP_TOPICS) {
      const section = document.getElementById(anchorOf(topic));
      expect(section).not.toBeNull();
      expect(section?.textContent ?? '').not.toContain('help.topic');
      expect((section?.textContent ?? '').length).toBeGreaterThan(80);
    }
  });

  it('leads with why this is not the ping the game shows', () => {
    // The single most important string in the application: without it the honest answer
    // looks like a wrong one.
    render(<HelpPage topic={null} />);

    const headings = screen.getAllByRole('heading', { level: 3 });
    expect(headings[0]).toHaveTextContent('Why this is not the ping your game shows');
  });

  it('marks the section the reader was sent to', () => {
    render(<HelpPage topic="freeze" />);

    expect(document.getElementById(anchorOf('freeze'))?.className).toContain('--opened');
    expect(document.getElementById(anchorOf('loss'))?.className).not.toContain('--opened');
  });

  it('keeps the paragraph breaks its bodies were written with', () => {
    // Inside a single `<p>` a blank line collapses to a space, which shipped the longest
    // topic as one unbroken ~350-word block.
    render(<HelpPage topic={null} />);

    const section = document.getElementById(anchorOf('network'));
    expect(section?.querySelectorAll('.nm-help-page__body').length).toBeGreaterThan(1);
  });

  it('promises nothing it would have to fetch', () => {
    // Bundled, not a website: an external link is a request this product promised never to
    // make on the user's behalf, and it is useless to someone being filtered.
    render(<HelpPage topic={null} />);

    expect(screen.queryAllByRole('link')).toHaveLength(0);
  });
});
