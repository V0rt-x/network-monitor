import type { CheckMarkView, ServiceGroup } from '../../shared/ipc';

/**
 * Maps the status page's IPC enums to i18next keys.
 *
 * Exhaustive switches on purpose: when Rust gains a variant, `tauri-specta` widens the
 * TypeScript union and these functions stop compiling — which is how a new state gets a
 * translation instead of silently rendering a raw identifier.
 */
export const serviceGroupKey = (group: ServiceGroup) => {
  switch (group) {
    case 'gamingPlatform':
      return 'status.group.gamingPlatform' as const;
    case 'infrastructure':
      return 'status.group.infrastructure' as const;
  }
};

export const serviceGroupHintKey = (group: ServiceGroup) => {
  switch (group) {
    case 'gamingPlatform':
      return 'status.groupHint.gamingPlatform' as const;
    case 'infrastructure':
      return 'status.groupHint.infrastructure' as const;
  }
};

export const checkMarkKey = (mark: CheckMarkView) => {
  switch (mark) {
    case 'answered':
      return 'status.check.answered' as const;
    case 'slow':
      return 'status.check.slow' as const;
    case 'lost':
      return 'status.check.lost' as const;
    case 'refused':
      return 'status.check.refused' as const;
    case 'filtered':
      return 'status.check.filtered' as const;
  }
};

/**
 * CSS modifier for one check on the timeline.
 *
 * Colour is a second channel and never the only one: every cell also carries its translated
 * word as a title and the strip has a text summary beside it, so the page stays readable
 * without colour vision.
 */
export const checkModifier = (mark: CheckMarkView) => `nm-check--${mark}` as const;
