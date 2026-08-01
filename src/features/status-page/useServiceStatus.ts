import { useEffect, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';

import { subscribeToServiceStatus, type ServiceStatus } from '../../shared/ipc';

/**
 * The status page's data, pushed by Rust.
 *
 * The UI never triggers a check: Rust decides the cadence and emits a snapshot at most once
 * a second, and nothing at all while the window is hidden. `unavailable` stays a distinct
 * state so a broken event channel cannot masquerade as a set of services nobody has got
 * around to checking yet — which on this page would look like calm.
 */
export type ServiceStatusState =
  | { readonly kind: 'waiting' }
  | { readonly kind: 'checking'; readonly status: ServiceStatus }
  | { readonly kind: 'unavailable' };

export const useServiceStatus = (): ServiceStatusState => {
  const [state, setState] = useState<ServiceStatusState>({ kind: 'waiting' });

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | undefined;

    void subscribeToServiceStatus((status) => {
      setState({ kind: 'checking', status });
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
