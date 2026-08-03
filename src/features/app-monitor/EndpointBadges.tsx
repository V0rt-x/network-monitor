import { useTranslation } from 'react-i18next';

import type { EndpointView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';

interface EndpointBadgesProps {
  readonly endpoint: EndpointView;
}

/**
 * Everything about an endpoint that stays at level one *in words*.
 *
 * Only warnings are left here, and the test for one is whether there is something to do about
 * it. A freeze is the strongest evidence on the page that something is wrong right now, and it
 * is the one figure here a player recognises immediately; an egress conflict says the figure
 * describes a different route from the one the application is taking; an endpoint nothing can
 * measure says the dashes are permanent. **A warning is never demoted, whatever a layout
 * costs**, so these keep their pill and their sentence while the state and its qualifiers
 * became marks beside them.
 */
export const EndpointBadges = ({ endpoint }: EndpointBadgesProps) => {
  const { t } = useTranslation();

  return (
    <>
      {/* Level one, everywhere, no exceptions: your application is still sending and nothing
          has come back for this long. */}
      {endpoint.flow?.stallMs != null && (
        <span className="nm-health nm-tone--unreachable">
          <MetricHelp topic="freeze">
            {t('apps.passive.stall', { ms: Math.round(endpoint.flow.stallMs) })}
          </MetricHelp>
        </span>
      )}
      {endpoint.egressConflict && (
        <span className="nm-badge nm-badge--warn">{t('apps.badge.egressConflict')}</span>
      )}
      {!endpoint.measurable && (
        <span className="nm-badge nm-badge--warn">{t('dashboard.badge.notMeasurable')}</span>
      )}
    </>
  );
};
