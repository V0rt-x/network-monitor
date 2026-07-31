import 'i18next';

import type common from '../locales/en/common.json';

/**
 * Makes every translation key type-checked: `t('does.not.exist')` is a compile error,
 * which is how CLAUDE.md's "no hardcoded user-visible strings" rule stays enforceable.
 */
declare module 'i18next' {
  interface CustomTypeOptions {
    defaultNS: 'common';
    resources: { common: typeof common };
  }
}
