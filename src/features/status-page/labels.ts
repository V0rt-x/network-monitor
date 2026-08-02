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

/**
 * Every state a cell can be in, in the order the legend names them.
 *
 * Worst to best rather than in the enum's order: a reader scanning the legend for the colour
 * they can see on a card is looking for a problem, not for the colour of "answered".
 * Exhaustive by construction — the `satisfies` makes a new Rust variant a compile error
 * here, so a state can never reach the page without a place in the legend that explains it.
 */
export const CHECK_MARKS = [
  'lost',
  'filtered',
  'refused',
  'slow',
  'answered',
] as const satisfies readonly CheckMarkView[];

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
 * Colour is a second channel and never the only one: every cell carries its translated word
 * for a screen reader, pointing at one or arrowing through the strip writes that word out
 * under it, and the page's legend names all five states beside their colours. A reader who
 * sees neither green nor red loses nothing.
 */
export const checkModifier = (mark: CheckMarkView) => `nm-check--${mark}` as const;
