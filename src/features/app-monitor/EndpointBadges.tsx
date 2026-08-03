import { useTranslation } from 'react-i18next';

import type { EndpointView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';

interface EndpointBadgesProps {
  readonly endpoint: EndpointView;
}

/**
 * Everything about an endpoint that stays at level one whatever else moves down.
 *
 * Two kinds of thing, and both earn their place by the same test — is there something to do
 * about it, or does it change what the figures beside it mean:
 *
 * **Warnings.** A freeze is the strongest evidence on the page that something is wrong right
 * now, and it is the one figure here a player recognises immediately; an egress conflict says
 * the figure describes a different route from the one the application is taking; an endpoint
 * nothing can measure says the dashes are permanent. A warning is never demoted, whatever a
 * layout costs, so these appear on a table row exactly as they did on a card.
 *
 * **Qualifiers that change the reading.** A tunnel makes the figure end-to-end through it
 * rather than a round trip to the server; a warm-up says the window is not full yet. A reader
 * who never opens the expander would otherwise read a tunnel's round trip as the server's.
 */
export const EndpointBadges = ({ endpoint }: EndpointBadgesProps) => {
  const { t } = useTranslation();

  return (
    <>
      {/* Level one, everywhere, no exceptions: your application is still sending and nothing
          has come back for this long. */}
      {endpoint.flow?.stallMs != null && (
        <span className="nm-health nm-health--unreachable">
          <MetricHelp topic="freeze">
            {t('apps.passive.stall', { ms: Math.round(endpoint.flow.stallMs) })}
          </MetricHelp>
        </span>
      )}
      {/* Said out loud, with the time left, rather than shown as dashes that read like a
          failure. Rust decides when it is over. */}
      {endpoint.warmupSecsRemaining !== null && (
        <span className="nm-badge">
          {t('apps.warmup.badge', { seconds: Math.ceil(endpoint.warmupSecsRemaining) })}
        </span>
      )}
      {endpoint.egressConflict && (
        <span className="nm-badge nm-badge--warn">{t('apps.badge.egressConflict')}</span>
      )}
      {!endpoint.measurable && (
        <span className="nm-badge nm-badge--warn">{t('dashboard.badge.notMeasurable')}</span>
      )}
      {endpoint.tunnelled && (
        <span className="nm-badge">
          <MetricHelp topic="tunnel">{t('dashboard.badge.tunnelled')}</MetricHelp>
        </span>
      )}
    </>
  );
};
