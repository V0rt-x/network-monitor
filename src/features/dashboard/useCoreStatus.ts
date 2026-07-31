import { useEffect, useState } from 'react';

import { fetchCoreStatus, type CoreStatus } from '../../shared/ipc';

/**
 * What the UI knows about the Rust core right now. `unreachable` is a real, displayed
 * state — never a blank panel that looks like "everything is fine".
 */
export type CoreStatusState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly status: CoreStatus }
  | { readonly kind: 'unreachable' };

export const useCoreStatus = (): CoreStatusState => {
  const [state, setState] = useState<CoreStatusState>({ kind: 'loading' });

  useEffect(() => {
    let active = true;

    void fetchCoreStatus().then(
      (status) => {
        if (active) setState({ kind: 'ready', status });
      },
      () => {
        if (active) setState({ kind: 'unreachable' });
      },
    );

    return () => {
      active = false;
    };
  }, []);

  return state;
};
