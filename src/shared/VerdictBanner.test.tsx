import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import '../i18n';
import type { DiagnosisView } from './ipc';
import { VerdictBanner } from './VerdictBanner';

const diagnosis = (overrides: Partial<DiagnosisView> = {}): DiagnosisView => ({
  verdict: 'clear',
  actionable: false,
  endpointsAffected: 0,
  endpointsTotal: 0,
  ...overrides,
});

describe('VerdictBanner', () => {
  it('says it does not know yet rather than saying nothing', () => {
    // A banner that appeared only on bad news would make its absence mean "fine", and the
    // state before anything has been measured would read as good news.
    render(<VerdictBanner diagnosis={diagnosis({ verdict: 'notEnoughEvidence' })} />);
    expect(screen.getByText('Not enough measured yet to say')).toBeInTheDocument();
  });

  it('does not interrupt a screen reader with a state of not knowing', () => {
    render(<VerdictBanner diagnosis={diagnosis({ verdict: 'notEnoughEvidence' })} />);
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('announces a finding', () => {
    render(
      <VerdictBanner diagnosis={diagnosis({ verdict: 'crossBorderPath', actionable: true })} />,
    );
    expect(screen.getByRole('status')).toBeInTheDocument();
  });

  it('names what the verdict is about when it is about one application', () => {
    render(
      <VerdictBanner
        diagnosis={diagnosis({ verdict: 'routeToThisApplication', actionable: true })}
        subject="Apex Legends"
      />,
    );
    expect(screen.getByText(/^Apex Legends:/u)).toBeInTheDocument();
  });

  it('says how many endpoints a verdict covers', () => {
    // "Two of seven endpoints" is a different message from "your game is unreachable", and
    // partial failure inside one application is the normal case under filtering.
    render(
      <VerdictBanner
        diagnosis={diagnosis({
          verdict: 'routeToThisApplication',
          actionable: true,
          endpointsAffected: 2,
          endpointsTotal: 7,
        })}
      />,
    );
    expect(screen.getByText('About 2 of 7 endpoints')).toBeInTheDocument();
  });

  it('does not claim a scope for a verdict about the general network', () => {
    const { container } = render(
      <VerdictBanner diagnosis={diagnosis({ verdict: 'crossBorderPath', actionable: true })} />,
    );
    expect(container.querySelector('.nm-verdict__scope')).toBeNull();
  });

  it('separates what to try from what was observed', () => {
    render(
      <VerdictBanner diagnosis={diagnosis({ verdict: 'crossBorderPath', actionable: true })} />,
    );
    expect(screen.getByText(/A VPN may help/u)).toBeInTheDocument();
  });

  it('offers no advice where there is nothing to advise', () => {
    for (const verdict of ['clear', 'notEnoughEvidence', 'nothingMeasurable'] as const) {
      const { container, unmount } = render(<VerdictBanner diagnosis={diagnosis({ verdict })} />);
      expect(container.querySelector('.nm-verdict__advice')).toBeNull();
      unmount();
    }
  });

  it('never suggests a VPN for a failure inside the user own network', () => {
    // Sending someone to a VPN when their own line is broken wastes their time and, in some
    // places, exposes them for nothing.
    render(
      <VerdictBanner
        diagnosis={diagnosis({ verdict: 'localNetworkOrProvider', actionable: true })}
      />,
    );
    expect(screen.getByText(/a VPN will not help/u)).toBeInTheDocument();
  });

  it('states a game-server verdict as a fact about reachability from here', () => {
    render(
      <VerdictBanner
        diagnosis={diagnosis({ verdict: 'gameServersUnreachable', actionable: true })}
      />,
    );
    // Never "the game is down": the app cannot observe that, only what answers from here.
    expect(
      screen.getByText("The game's own reference servers are not answering from here"),
    ).toBeInTheDocument();
  });

  it('carries no evidence expander when none is given', () => {
    // Every caller but the Network page has no baselines of its own to show, and an empty
    // expander would be a second inventory beside the claim with nothing in it.
    render(<VerdictBanner diagnosis={diagnosis()} />);
    expect(screen.queryByText('What this is drawn from')).not.toBeInTheDocument();
  });

  it('folds the evidence it is given behind its own expander', () => {
    render(<VerdictBanner diagnosis={diagnosis()} evidence={<p>Domestic and foreign, here</p>} />);

    const expander = screen.getByText('What this is drawn from').closest('details');
    if (!(expander instanceof HTMLDetailsElement)) {
      throw new Error('the evidence is not a folding element');
    }
    expect(expander).not.toHaveAttribute('open');
    expect(within(expander).getByText('Domestic and foreign, here')).toBeInTheDocument();
  });
});
