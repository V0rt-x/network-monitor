import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';

import { registerTrayLabels } from '../../shared/ipc';

/**
 * Keeps the tray menu in the UI's language.
 *
 * The tray is built in Rust but its words come from here, because every user-visible string
 * in this product goes through an i18next key and those live in the frontend. Rust starts
 * the tray with an icon and no menu; this hook supplies the menu on mount and again
 * whenever the language changes, so adding a locale stays what CLAUDE.md promises — new
 * JSON, no code.
 *
 * Until it succeeds the app has no way back from the tray, which is why Rust treats closing
 * the window as a real quit until the menu exists.
 */
export const useTrayLabels = (): void => {
  const { t, i18n } = useTranslation();
  const language = i18n.language;

  useEffect(() => {
    // A tray menu that could not be built is visible as its own absence; there is nothing
    // useful to tell the user about it, and Rust has already fallen back to quitting on
    // close rather than hiding.
    void registerTrayLabels({ show: t('tray.show'), quit: t('tray.quit') }).catch(() => undefined);
  }, [t, language]);
};
