import { useEffect, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';

import { subscribeToAppEndpoints, type AppEndpoints } from '../../shared/ipc';

/**
 * What every monitored application is talking to, pushed by Rust.
 *
 * The UI never polls and never asks for a sample. `unavailable` stays a distinct state so a
 * broken event channel cannot masquerade as "you have not chosen an application yet" —
 * those look identical on screen and mean entirely different things.
 */
export type AppEndpointsState =
  | { readonly kind: 'waiting' }
  | { readonly kind: 'measuring'; readonly endpoints: AppEndpoints }
  | { readonly kind: 'unavailable' };

export const useAppEndpoints = (): AppEndpointsState => {
  const [state, setState] = useState<AppEndpointsState>({ kind: 'waiting' });

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | undefined;

    void subscribeToAppEndpoints((endpoints) => {
      setState({ kind: 'measuring', endpoints });
    }).then(
      (stop) => {
        if (active) unlisten = stop;
        else stop();
      },
      () => {
        if (active) setState({ kind: 'unavailable' });
      },
    );

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return state;
};
