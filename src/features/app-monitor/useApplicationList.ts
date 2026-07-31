import { useCallback, useEffect, useState } from 'react';

import { fetchApplications, type ApplicationListView } from '../../shared/ipc';

/**
 * The applications the picker may offer.
 *
 * Fetched when the picker opens and when the user asks again — never on a timer. The list
 * is a snapshot the moment it is taken, so polling one would spend budget to be no less
 * stale, and Rust re-checks the process when monitoring actually starts.
 */
export type ApplicationListState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'listed'; readonly list: ApplicationListView }
  | { readonly kind: 'unavailable' };

export interface ApplicationList {
  readonly state: ApplicationListState;
  readonly refresh: () => void;
}

export const useApplicationList = (): ApplicationList => {
  const [state, setState] = useState<ApplicationListState>({ kind: 'loading' });
  const [attempt, setAttempt] = useState(0);

  const refresh = useCallback(() => {
    setAttempt((previous) => previous + 1);
  }, []);

  useEffect(() => {
    let active = true;
    setState({ kind: 'loading' });

    void fetchApplications().then(
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
