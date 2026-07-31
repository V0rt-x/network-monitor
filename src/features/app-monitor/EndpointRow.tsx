import { useTranslation } from 'react-i18next';

import { formatBytes, formatMs, formatPct } from '../../shared/format';
import type { EndpointView } from '../../shared/ipc';
import { healthKey, healthModifier, probeKindKey } from '../dashboard/labels';
import { Sparkline } from '../dashboard/Sparkline';
import { livenessKey, probingKey, transportKey } from './labels';

interface EndpointRowProps {
  readonly endpoint: EndpointView;
  /** Span the byte count covers, so the traffic figure can say what it is a count of. */
  readonly trafficWindowSecs: number;
}

/**
 * One endpoint of one application: its own verdict, its own numbers, its own caveats.
 *
 * Nothing here is rolled up to the application. Within one game some endpoints stay clean
 * while others lose packets or go unreachable — a login service on a CDN, voice on one
 * provider, the match server on another — and a single colour for the application would
 * either report a working game as broken or hide the failure the user came to find.
 *
 * The caveats are not decoration either. A figure produced by a TLS hello is not the same
 * quantity as an ICMP round trip; a figure measured through a local tunnel is not a round
 * trip to the server at all; an endpoint whose probe egresses somewhere other than the
 * application's own flow is not measuring the route the user is asking about. Each is
 * stated beside the number rather than folded into it.
 *
 * The route caveat covers two cases that look the same to the user and are: another
 * monitored application reaching the endpoint by a different interface, and an address a
 * baseline was already probing, whose binding was chosen before this application asked.
 * Either way the single probe cannot be promised to follow this application's route.
 */
export const EndpointRow = ({ endpoint, trafficWindowSecs }: EndpointRowProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  return (
    <li className="nm-endpoint">
      <div className="nm-endpoint__identity">
        <span className="nm-endpoint__address">{endpoint.address}</span>
        <span className="nm-endpoint__transport">{t(transportKey(endpoint.transport))}</span>
      </div>

      <div className="nm-endpoint__badges">
        <span className={`nm-health ${healthModifier(endpoint.health)}`}>
          {t(healthKey(endpoint.health))}
        </span>
        {endpoint.probeKind !== null && (
          <span className="nm-badge">{t(probeKindKey(endpoint.probeKind))}</span>
        )}
        {endpoint.liveness === 'idle' && (
          <span className="nm-badge">{t(livenessKey(endpoint.liveness))}</span>
        )}
        {endpoint.probing === 'demoted' && (
          <span className="nm-badge">{t(probingKey(endpoint.probing))}</span>
        )}
        {endpoint.tunnelled && <span className="nm-badge">{t('dashboard.badge.tunnelled')}</span>}
        {endpoint.filteringConfirmed && (
          <span className="nm-badge">{t('dashboard.badge.filteringConfirmed')}</span>
        )}
        {endpoint.egressConflict && (
          <span className="nm-badge nm-badge--warn">{t('apps.badge.egressConflict')}</span>
        )}
        {!endpoint.measurable && (
          <span className="nm-badge nm-badge--warn">{t('dashboard.badge.notMeasurable')}</span>
        )}
      </div>

      <dl className="nm-endpoint__metrics">
        <div>
          <dt>{t('dashboard.metric.rtt')}</dt>
          <dd>{formatMs(endpoint.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.jitter')}</dt>
          <dd>{formatMs(endpoint.jitterMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.loss')}</dt>
          <dd>{formatPct(endpoint.lossPct, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.metric.traffic', { seconds: trafficWindowSecs })}</dt>
          <dd>{formatBytes(endpoint.recentBytes, locale)}</dd>
        </div>
      </dl>

      <p className="nm-endpoint__egress">
        {endpoint.egress === null
          ? t('apps.egress.unknown')
          : t('apps.egress.via', { address: endpoint.egress })}
      </p>

      <Sparkline
        ageSecs={endpoint.seriesAgeSecs}
        rttMs={endpoint.seriesRttMs}
        health={endpoint.health}
        label={t('apps.sparklineLabel', { endpoint: endpoint.address })}
      />
    </li>
  );
};
