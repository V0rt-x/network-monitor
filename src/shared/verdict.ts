import type { VerdictView } from './ipc';

/**
 * Maps a verdict to its i18next key.
 *
 * An exhaustive switch on purpose: when Rust gains a verdict, `tauri-specta` widens the
 * TypeScript union and this stops compiling — which is how a new conclusion gets a
 * translation instead of silently rendering a raw identifier at the one place in the app
 * where the wording matters most.
 */
export const verdictKey = (verdict: VerdictView) => {
  switch (verdict) {
    case 'notEnoughEvidence':
      return 'verdict.notEnoughEvidence' as const;
    case 'nothingMeasurable':
      return 'verdict.nothingMeasurable' as const;
    case 'clear':
      return 'verdict.clear' as const;
    case 'localNetworkOrProvider':
      return 'verdict.localNetworkOrProvider' as const;
    case 'crossBorderPath':
      return 'verdict.crossBorderPath' as const;
    case 'routeToThisApplication':
      return 'verdict.routeToThisApplication' as const;
    case 'gameServersUnreachable':
      return 'verdict.gameServersUnreachable' as const;
    case 'gameServersPartlyUnreachable':
      return 'verdict.gameServersPartlyUnreachable' as const;
  }
};

/**
 * The key for what the user can do about a verdict.
 *
 * Kept apart from the verdict's own wording because they are different claims. The verdict
 * is what was observed; this is a suggestion, and it is phrased as one — the app cannot see
 * whether a VPN would help, only that the evidence points past the border.
 */
export const verdictAdviceKey = (verdict: VerdictView) => {
  switch (verdict) {
    case 'localNetworkOrProvider':
      return 'verdict.advice.localNetworkOrProvider' as const;
    case 'crossBorderPath':
      return 'verdict.advice.crossBorderPath' as const;
    case 'routeToThisApplication':
      return 'verdict.advice.routeToThisApplication' as const;
    case 'gameServersUnreachable':
    case 'gameServersPartlyUnreachable':
      return 'verdict.advice.gameServers' as const;
    // Nothing to advise: these are states of not knowing, or of everything being fine.
    case 'notEnoughEvidence':
    case 'nothingMeasurable':
    case 'clear':
      return null;
  }
};

/** CSS modifier for a verdict, so the banner reads at a glance as well as in words. */
export const verdictModifier = (verdict: VerdictView) => `nm-verdict--${verdict}` as const;
