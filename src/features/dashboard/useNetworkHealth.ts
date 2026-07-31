import { useEffect, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';

import { subscribeToNetworkHealth, type NetworkHealth } from '../../shared/ipc';

/**
 * The general-health picture, pushed by Rust.
 *
 * The UI never polls and never asks for a sample: Rust owns the schedule and emits at most
 * once a second, and nothing at all while the window is hidden. `unavailable` stays a
 * distinct state so a broken event channel cannot masquerade as a network that has simply
 * not been measured yet.
 */
export type NetworkHealthState =
  | { readonly kind: 'waiting' }
  | { readonly kind: 'measuring'; readonly health: NetworkHealth }
  | { readonly kind: 'unavailable' };

export const useNetworkHealth = (): NetworkHealthState => {
  const [state, setState] = useState<NetworkHealthState>({ kind: 'waiting' });

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | undefined;

    void subscribeToNetworkHealth((health) => {
      setState({ kind: 'measuring', health });
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
