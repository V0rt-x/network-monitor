import type { CoreReadiness, PlatformKind } from '../../shared/ipc';

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
