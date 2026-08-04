import { useEffect, useState } from 'react';

import { fetchNetworkCatalogue, type NetworkCatalogueView } from '../../shared/ipc';

/**
 * The bundled catalogue an edit chooser may offer, fetched once.
 *
 * Unlike the application picker's list, this never goes stale during a session: it is
 * compiled into the binary and changes only with a release, so there is nothing here a
 * refresh button could usefully ask for.
 */
export type NetworkCatalogueState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'listed'; readonly catalogue: NetworkCatalogueView }
  | { readonly kind: 'unavailable' };

export const useNetworkCatalogue = (): NetworkCatalogueState => {
  const [state, setState] = useState<NetworkCatalogueState>({ kind: 'loading' });

  useEffect(() => {
    let active = true;

    void fetchNetworkCatalogue().then(
      (catalogue) => {
        if (active) setState({ kind: 'listed', catalogue });
      },
      () => {
        if (active) setState({ kind: 'unavailable' });
      },
    );

    return () => {
      active = false;
    };
  }, []);

  return state;
};
