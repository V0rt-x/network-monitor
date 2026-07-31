import type {
  BaselineGroup,
  CoreReadiness,
  HealthView,
  PlatformKind,
  ProbeKindView,
  SettingsProblem,
} from '../../shared/ipc';

/**
 * Maps IPC enums to i18next keys.
 *
 * Exhaustive switches on purpose: when Rust gains a variant, `tauri-specta` widens the
 * TypeScript union and these functions stop compiling — which is how a new state gets a
 * translation instead of silently rendering a raw identifier.
 */
export const readinessKey = (readiness: CoreReadiness) => {
  switch (readiness) {
    case 'ready':
      return 'core.readiness.ready' as const;
    case 'unsupportedPlatform':
      return 'core.readiness.unsupportedPlatform' as const;
  }
};

export const platformKey = (platform: PlatformKind) => {
  switch (platform) {
    case 'windows':
      return 'core.platform.windows' as const;
    case 'linux':
      return 'core.platform.linux' as const;
    case 'macOs':
      return 'core.platform.macOs' as const;
    case 'unsupported':
      return 'core.platform.unsupported' as const;
  }
};

export const groupKey = (group: BaselineGroup) => {
  switch (group) {
    case 'domestic':
      return 'dashboard.group.domestic' as const;
    case 'foreign':
      return 'dashboard.group.foreign' as const;
  }
};

export const groupHintKey = (group: BaselineGroup) => {
  switch (group) {
    case 'domestic':
      return 'dashboard.groupHint.domestic' as const;
    case 'foreign':
      return 'dashboard.groupHint.foreign' as const;
  }
};

export const healthKey = (health: HealthView) => {
  switch (health) {
    case 'ok':
      return 'dashboard.health.ok' as const;
    case 'degraded':
      return 'dashboard.health.degraded' as const;
    case 'unreachable':
      return 'dashboard.health.unreachable' as const;
    case 'blocked':
      return 'dashboard.health.blocked' as const;
    case 'unknown':
      return 'dashboard.health.unknown' as const;
  }
};

export const probeKindKey = (kind: ProbeKindView) => {
  switch (kind) {
    case 'icmpEcho':
      return 'dashboard.probeKind.icmpEcho' as const;
    case 'tcpConnect':
      return 'dashboard.probeKind.tcpConnect' as const;
    case 'tlsHello':
      return 'dashboard.probeKind.tlsHello' as const;
  }
};

export const settingsProblemKey = (problem: SettingsProblem) => {
  switch (problem) {
    case 'unreadable':
      return 'settings.problem.unreadable' as const;
    case 'malformed':
      return 'settings.problem.malformed' as const;
    case 'notWritable':
      return 'settings.problem.notWritable' as const;
  }
};

/**
 * CSS modifier for a health state.
 *
 * Colour is a second channel, never the only one — every state also carries its translated
 * word, so the dashboard stays readable without colour vision.
 */
export const healthModifier = (health: HealthView) => `nm-health--${health}` as const;
