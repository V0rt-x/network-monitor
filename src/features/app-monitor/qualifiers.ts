import { useTranslation } from 'react-i18next';

import type { EndpointView } from '../../shared/ipc';
import type { Qualifier } from '../../shared/StateToken';

/**
 * The qualifiers an endpoint carries — what changes the meaning of the figures beside it.
 *
 * A tunnel makes a round trip end-to-end through it rather than a round trip to the server; a
 * warm-up says the window is not full yet. Neither is a fault and there is nothing to do
 * about either, so they travel with the state token as marks rather than as pills — they were
 * two of the three bordered words that took more width on a row than every figure combined.
 *
 * A reader who never opens an expander still meets them: the token's accessible name has them
 * at all times, and hovering or focusing it spells them out.
 *
 * They are deliberately *not* warnings. The test for a warning is whether there is something
 * to do about it, and a freeze, an egress conflict or an endpoint nothing can measure keep
 * their words on the row precisely because there is.
 */
export const useQualifiers = (endpoint: EndpointView): readonly Qualifier[] => {
  const { t } = useTranslation();
  const qualifiers: Qualifier[] = [];
  if (endpoint.warmupSecsRemaining !== null) {
    qualifiers.push({
      kind: 'warmup',
      name: t('apps.warmup.badge', { seconds: Math.ceil(endpoint.warmupSecsRemaining) }),
    });
  }
  if (endpoint.tunnelled) {
    qualifiers.push({ kind: 'tunnelled', name: t('dashboard.badge.tunnelled') });
  }
  return qualifiers;
};
