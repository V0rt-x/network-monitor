import type { TFunction } from 'i18next';

import type { NetworkView } from '../../shared/ipc';

/**
 * What to call a network on the page.
 *
 * The registered name where the directory has one, and the autonomous system number where it
 * does not — `AS13335` is not friendly, but it is *true*, it is searchable, and it is the
 * identity the rest of the networking world uses for the same thing. The alternative would be
 * to show nothing for an address whose network is known but unnamed, which throws away a fact
 * the reader could have used.
 *
 * In a module of its own so the endpoint row and the route panel name a hop by one rule; two
 * copies would drift the day one of them gained a fallback the other did not.
 */
export const networkName = (network: NetworkView, t: TFunction): string =>
  network.name ?? t('apps.network.unnamed', { asn: network.asn });
