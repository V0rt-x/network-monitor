import i18next from 'i18next';
import { initReactI18next } from 'react-i18next';

import common from '../locales/en/common.json';

/** Only namespace for now; feature namespaces are added as data, never as code paths. */
export const defaultNS = 'common';

/**
 * Locale resources are bundled, not fetched. The app makes no network request the user
 * did not ask for, and that includes loading its own translations.
 */
export const resources = { en: { common } } as const;

void i18next.use(initReactI18next).init({
  resources,
  lng: 'en',
  fallbackLng: 'en',
  defaultNS,
  ns: [defaultNS],
  // React already escapes interpolated values.
  interpolation: { escapeValue: false },
});

export { i18next };
