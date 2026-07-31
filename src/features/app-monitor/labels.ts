import type {
  ApplicationListProblem,
  FlowStatusView,
  LivenessView,
  PathPositionView,
  PathQualityView,
  ProbingView,
  TransportView,
} from '../../shared/ipc';

/**
 * Maps the app-monitor IPC enums to i18next keys.
 *
 * Exhaustive switches on purpose: when Rust gains a variant, `tauri-specta` widens the
 * TypeScript union and these functions stop compiling — which is how a new state gets a
 * translation instead of silently rendering a raw identifier.
 */
export const transportKey = (transport: TransportView) => {
  switch (transport) {
    case 'tcp':
      return 'apps.transport.tcp' as const;
    case 'udp':
      return 'apps.transport.udp' as const;
  }
};

export const livenessKey = (liveness: LivenessView) => {
  switch (liveness) {
    case 'active':
      return 'apps.liveness.active' as const;
    case 'idle':
      return 'apps.liveness.idle' as const;
  }
};

export const probingKey = (probing: ProbingView) => {
  switch (probing) {
    case 'active':
      return 'apps.probing.active' as const;
    case 'demoted':
      return 'apps.probing.demoted' as const;
  }
};

export const flowStatusKey = (status: FlowStatusView) => {
  switch (status) {
    case 'active':
      return 'apps.flow.active' as const;
    case 'notPermitted':
      return 'apps.flow.notPermitted' as const;
    case 'stopped':
      return 'apps.flow.stopped' as const;
    case 'unavailable':
      return 'apps.flow.unavailable' as const;
  }
};

export const pathQualityKey = (quality: PathQualityView) => {
  switch (quality) {
    case 'notMeasuredYet':
      return 'apps.path.quality.notMeasuredYet' as const;
    case 'ok':
      return 'apps.path.quality.ok' as const;
    case 'degraded':
      return 'apps.path.quality.degraded' as const;
    case 'uncorroborated':
      return 'apps.path.quality.uncorroborated' as const;
    case 'lost':
      return 'apps.path.quality.lost' as const;
  }
};

export const pathPositionKey = (position: PathPositionView) => {
  switch (position) {
    case 'reached':
      return 'apps.path.position.reached' as const;
    case 'nothingAnswered':
      return 'apps.path.position.nothingAnswered' as const;
    case 'insideThisNetwork':
      return 'apps.path.position.insideThisNetwork' as const;
    case 'insideTheAccessNetwork':
      return 'apps.path.position.insideTheAccessNetwork' as const;
    case 'beforeAnyLongHaulLink':
      return 'apps.path.position.beforeAnyLongHaulLink' as const;
    case 'beyondALongHaulLink':
      return 'apps.path.position.beyondALongHaulLink' as const;
  }
};

/** How a path verdict is shown, reusing the health palette rather than inventing one. */
export const pathQualityModifier = (quality: PathQualityView) => {
  switch (quality) {
    case 'ok':
      return 'nm-health--ok' as const;
    case 'degraded':
    case 'lost':
      return 'nm-health--degraded' as const;
    case 'uncorroborated':
    case 'notMeasuredYet':
      return 'nm-health--unknown' as const;
  }
};

export const applicationProblemKey = (problem: ApplicationListProblem) => {
  switch (problem) {
    case 'unsupportedPlatform':
      return 'apps.picker.unsupportedPlatform' as const;
    case 'refused':
      return 'apps.picker.refused' as const;
  }
};
