import { useCallback, useEffect, useState } from 'react';

import { fetchSettings, storeSettings, type Settings, type SettingsView } from '../../shared/ipc';

/**
 * The settings in force, and a way to change them.
 *
 * Every write goes to Rust and the *reply* becomes the new state — never the value that
 * was sent. Rust clamps intervals, rejects unknown countries and reports what the platform
 * actually did about autostart, so echoing the request locally would show the user a
 * setting that is not in effect.
 */
export type SettingsState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly view: SettingsView; readonly saving: boolean }
  | { readonly kind: 'unavailable' };

export interface SettingsController {
  readonly state: SettingsState;
  /** Applies a change to the settings currently in force. */
  readonly change: (patch: Partial<Settings>) => void;
}

export const useSettings = (): SettingsController => {
  const [state, setState] = useState<SettingsState>({ kind: 'loading' });

  useEffect(() => {
    let active = true;

    void fetchSettings().then(
      (view) => {
        if (active) setState({ kind: 'ready', view, saving: false });
      },
      () => {
        if (active) setState({ kind: 'unavailable' });
      },
    );

    return () => {
      active = false;
    };
  }, []);

  const change = useCallback((patch: Partial<Settings>) => {
    setState((current) => {
      if (current.kind !== 'ready') return current;

      const wanted: Settings = { ...current.view.settings, ...patch };
      void storeSettings(wanted).then(
        (view) => {
          setState({ kind: 'ready', view, saving: false });
        },
        () => {
          setState({ kind: 'unavailable' });
        },
      );

      return { ...current, saving: true };
    });
  }, []);

  return { state, change };
};
