import type { Section } from '../../shared/ipc';

/**
 * Maps the Network page's IPC enums to i18next keys.
 *
 * Exhaustive switches on purpose: when Rust gains a variant, `tauri-specta` widens the
 * TypeScript union and this stops compiling — which is how a new section gets a translation
 * instead of silently rendering a raw identifier.
 */
export const sectionKey = (section: Section) => {
  switch (section) {
    case 'domestic':
      return 'network.section.domestic' as const;
    case 'foreign':
      return 'network.section.foreign' as const;
    case 'gamingPlatform':
      return 'network.section.gamingPlatform' as const;
    case 'infrastructure':
      return 'network.section.infrastructure' as const;
    case 'other':
      return 'network.section.other' as const;
  }
};
