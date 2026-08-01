import { useTranslation } from 'react-i18next';

import { healthKey, healthModifier, probeKindKey } from '../dashboard/labels';
import { formatCount, formatMs, formatPct, NO_VALUE } from '../../shared/format';
import type { ServiceEndpointView, ServiceView } from '../../shared/ipc';
import { CheckTimeline } from './CheckTimeline';

interface ServiceCardProps {
  readonly service: ServiceView;
  readonly checkIntervalSecs: number;
}

/** Which counts are worth showing, and in what order of severity. */
const DISTRIBUTION = [
  { key: 'unreachable', labelKey: 'dashboard.health.unreachable' },
  { key: 'blocked', labelKey: 'dashboard.health.blocked' },
  { key: 'degraded', labelKey: 'dashboard.health.degraded' },
  { key: 'ok', labelKey: 'dashboard.health.ok' },
  { key: 'unknown', labelKey: 'dashboard.health.unknown' },
] as const;

interface EndpointRowProps {
  readonly endpoint: ServiceEndpointView;
  readonly serviceLabel: string;
  readonly alone: boolean;
}

/**
 * One endpoint of a service: its strip, its latency, and every caveat attached to them.
 *
 * The caveats are not decoration. A figure measured through a local tunnel is not a round
 * trip to the service; a name that never resolved is a finding rather than a blank; and
 * "we proved this probe kind is filtered" is a different claim from "this host is silent".
 */
const EndpointRow = ({ endpoint, serviceLabel, alone }: EndpointRowProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  return (
    <li className="nm-service__endpoint">
      <div className="nm-service__endpointhead">
        <span className="nm-service__address" title={endpoint.resolvedAddress ?? undefined}>
          {endpoint.writtenAddress}
        </span>
        <div className="nm-endpoint__badges">
          {/* A service with one endpoint already says its state in the headline; repeating
              it on the only row underneath would be noise. */}
          {!alone && (
            <span className={`nm-health ${healthModifier(endpoint.health)}`}>
              {t(healthKey(endpoint.health))}
            </span>
          )}
          {endpoint.probeKind !== null && (
            <span className="nm-badge">{t(probeKindKey(endpoint.probeKind))}</span>
          )}
          {endpoint.tunnelled && <span className="nm-badge">{t('dashboard.badge.tunnelled')}</span>}
          {endpoint.filteringConfirmed && (
            <span className="nm-badge">{t('dashboard.badge.filteringConfirmed')}</span>
          )}
          {endpoint.resolvedAddress === null && (
            <span className="nm-badge nm-badge--warn">{t('dashboard.badge.unresolved')}</span>
          )}
          {endpoint.resolvedAddress !== null && !endpoint.measurable && (
            <span className="nm-badge nm-badge--warn">{t('dashboard.badge.notMeasurable')}</span>
          )}
        </div>
      </div>

      <CheckTimeline
        checks={endpoint.checks}
        label={t('status.timelineLabel', {
          service: serviceLabel,
          address: endpoint.writtenAddress,
        })}
      />

      <dl className="nm-service__metrics">
        <div>
          <dt>{t('status.metric.latest')}</dt>
          <dd>{formatMs(endpoint.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('status.metric.mean')}</dt>
          <dd>{formatMs(endpoint.meanRttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('status.metric.loss')}</dt>
          <dd>{formatPct(endpoint.lossPct, locale)}</dd>
        </div>
      </dl>
    </li>
  );
};

/**
 * One service on the status page.
 *
 * The card answers *is it them or me*, and it is careful about which. What it reports is
 * whether this machine can reach the operator's published front door — a service the app
 * cannot reach may be perfectly healthy for everyone else, which is exactly what a user in
 * a filtered network needs to be able to tell. Nothing here says a company's service is
 * down.
 *
 * The distribution is shown whenever a service has more than one endpoint and they
 * disagree: a storefront answering while the gateway does not is the finding, and one
 * amber dot would hide which half is broken.
 */
export const ServiceCard = ({ service, checkIntervalSecs }: ServiceCardProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  const counts = DISTRIBUTION.map((entry) => ({
    ...entry,
    value: service.counts[entry.key],
  })).filter((entry) => entry.value > 0);

  const lastChecked =
    service.lastCheckedSecs === null
      ? t('status.neverChecked')
      : t('status.lastChecked', { seconds: Math.round(service.lastCheckedSecs) });
  // A check that is older than two whole intervals is not merely aging — something has
  // stopped. A status page whose data quietly stopped arriving looks exactly like one
  // reporting that everything is fine, so it says so instead.
  const stale = service.lastCheckedSecs !== null && service.lastCheckedSecs > checkIntervalSecs * 2;

  return (
    <section className="nm-service">
      <header className="nm-service__header">
        <h3 className="nm-service__title">{service.label}</h3>
        <span className={`nm-health ${healthModifier(service.verdict)}`}>
          {t(healthKey(service.verdict))}
        </span>
      </header>

      <p className="nm-service__meta">
        <span className="nm-service__rtt">
          {service.rttMs === null ? NO_VALUE : formatMs(service.rttMs, locale)}
        </span>
        <span className={stale ? 'nm-service__stale' : 'nm-service__checked'}>{lastChecked}</span>
      </p>

      {counts.length > 1 && (
        <ul className="nm-group__distribution">
          {counts.map((entry) => (
            <li key={entry.key} className={`nm-health ${healthModifier(entry.key)}`}>
              {t('dashboard.distributionEntry', {
                amount: formatCount(entry.value, locale),
                state: t(entry.labelKey),
              })}
            </li>
          ))}
        </ul>
      )}

      <ul className="nm-service__endpoints">
        {service.endpoints.map((endpoint) => (
          <EndpointRow
            key={endpoint.key}
            endpoint={endpoint}
            serviceLabel={service.label}
            alone={service.endpoints.length === 1}
          />
        ))}
      </ul>
    </section>
  );
};
