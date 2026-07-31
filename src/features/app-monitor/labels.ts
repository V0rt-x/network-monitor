import type {
  FlowStatusView,
  LivenessView,
  ProbingView,
  ProcessListProblem,
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

export const processProblemKey = (problem: ProcessListProblem) => {
  switch (problem) {
    case 'unsupportedPlatform':
      return 'apps.picker.unsupportedPlatform' as const;
    case 'refused':
      return 'apps.picker.refused' as const;
  }
};
