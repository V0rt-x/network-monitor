import type {
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
    case 'carryingTraffic':
      return 'dashboard.health.carryingTraffic' as const;
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
 * The colour a health state is drawn in — and only the colour.
 *
 * Deliberately not a shape and not a border: a count chip, a state token and a warning pill
 * are three different claims that share one palette, and the class that used to carry the
 * colour also carried a pill's border, which is how a count of endpoints in a state came to
 * look exactly like the state of one endpoint.
 *
 * Colour stays a second channel and never the only one: a token's *shape* differs too, a
 * count chip says its state in words, and a warning is a sentence.
 */
export const healthModifier = (health: HealthView) => `nm-tone--${health}` as const;
