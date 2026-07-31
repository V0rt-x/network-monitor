import { useEffect, useState } from 'react';
import type { UnlistenFn } from '@tauri-apps/api/event';

import { subscribeToHeartbeat, type CoreHeartbeat } from '../../shared/ipc';

/**
 * Liveness of the Rust -> UI event channel.
 *
 * `unavailable` means the subscription itself failed; it stays visible rather than
 * collapsing into `waiting`, so a broken channel cannot masquerade as a slow one.
 */
export type HeartbeatState =
  | { readonly kind: 'waiting' }
  | { readonly kind: 'beating'; readonly beat: CoreHeartbeat }
  | { readonly kind: 'unavailable' };

export const useCoreHeartbeat = (): HeartbeatState => {
  const [state, setState] = useState<HeartbeatState>({ kind: 'waiting' });

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | undefined;

    void subscribeToHeartbeat((beat) => {
      setState({ kind: 'beating', beat });
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
