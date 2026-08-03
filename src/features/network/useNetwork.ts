import { useEffect, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';

import { subscribeToNetwork, type NetworkSnapshot } from '../../shared/ipc';

/**
 * The Network page's data, pushed by Rust.
 *
 * One subscription where there were two. The UI never triggers a probe: Rust decides every
 * cadence and emits a snapshot at most once a second, and nothing at all while the window is
 * hidden. `unavailable` stays a distinct state so a broken event channel cannot masquerade
 * as a network nobody has got around to measuring yet — which on this page would look like
 * calm.
 */
export type NetworkState =
  | { readonly kind: 'waiting' }
  | { readonly kind: 'measuring'; readonly snapshot: NetworkSnapshot }
  | { readonly kind: 'unavailable' };

export const useNetwork = (): NetworkState => {
  const [state, setState] = useState<NetworkState>({ kind: 'waiting' });

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | undefined;

    void subscribeToNetwork((snapshot) => {
      setState({ kind: 'measuring', snapshot });
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
