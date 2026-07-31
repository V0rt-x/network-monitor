import { useCallback, useEffect, useState } from 'react';

import { fetchProcesses, type ProcessListView } from '../../shared/ipc';

/**
 * The processes the picker may offer.
 *
 * Fetched when the picker opens and when the user asks again — never on a timer. A process
 * list is a snapshot the moment it is taken, so polling one would spend budget to be no
 * less stale, and the identifier is re-checked by Rust when monitoring actually starts.
 */
export type ProcessListState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'listed'; readonly list: ProcessListView }
  | { readonly kind: 'unavailable' };

export interface ProcessList {
  readonly state: ProcessListState;
  readonly refresh: () => void;
}

export const useProcessList = (): ProcessList => {
  const [state, setState] = useState<ProcessListState>({ kind: 'loading' });
  const [attempt, setAttempt] = useState(0);

  const refresh = useCallback(() => {
    setAttempt((previous) => previous + 1);
  }, []);

  useEffect(() => {
    let active = true;
    setState({ kind: 'loading' });

    void fetchProcesses().then(
      (list) => {
        if (active) setState({ kind: 'listed', list });
      },
      () => {
        if (active) setState({ kind: 'unavailable' });
      },
    );

    return () => {
      active = false;
    };
  }, [attempt]);

  return { state, refresh };
};
