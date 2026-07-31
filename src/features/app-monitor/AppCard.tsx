import { useTranslation } from 'react-i18next';

import { formatCount } from '../../shared/format';
import type { AppView } from '../../shared/ipc';
import { healthModifier } from '../dashboard/labels';
import { EndpointRow } from './EndpointRow';

interface AppCardProps {
  readonly app: AppView;
  /** Span the byte counts cover. */
  readonly trafficWindowSecs: number;
  readonly onForget: (pid: number) => void;
}

/** Which counts are worth showing, and in what order of severity. */
const DISTRIBUTION = [
  { key: 'unreachable', labelKey: 'dashboard.health.unreachable' },
  { key: 'degraded', labelKey: 'dashboard.health.degraded' },
  { key: 'blocked', labelKey: 'dashboard.health.blocked' },
  { key: 'ok', labelKey: 'dashboard.health.ok' },
  { key: 'unknown', labelKey: 'dashboard.health.unknown' },
] as const;

/**
 * One monitored application: its endpoints, worst first, and the distribution across them.
 *
 * There is deliberately no verdict for the application as a whole. "4 clean, 2 degraded, 1
 * unreachable" is a fact the user can act on; one colour for a game is either an outage
 * that is not happening or a failure that is being hidden. Partial failure inside one
 * application is the normal case under filtering, not an edge case — its endpoints sit in
 * different networks and a tunnel may cover some of them and not others.
 */
export const AppCard = ({ app, trafficWindowSecs, onForget }: AppCardProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  const counts = DISTRIBUTION.map((entry) => ({
    ...entry,
    value: app.counts[entry.key],
  })).filter((entry) => entry.value > 0);

  return (
    <section className="nm-appcard">
      <header className="nm-appcard__header">
        <div>
          <h3 className="nm-appcard__title">{app.name}</h3>
          <p className="nm-appcard__pid">{t('apps.pid', { pid: app.pid })}</p>
        </div>
        <button
          type="button"
          className="nm-button nm-button--quiet"
          onClick={() => {
            onForget(app.pid);
          }}
        >
          {t('apps.stop')}
        </button>
      </header>

      {counts.length > 0 && (
        <ul className="nm-appcard__distribution">
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

      {app.endpoints.length === 0 ? (
        <p className="nm-state--pending">{t('apps.noEndpoints')}</p>
      ) : (
        <ul className="nm-appcard__endpoints">
          {app.endpoints.map((endpoint) => (
            <EndpointRow
              key={endpoint.key}
              endpoint={endpoint}
              trafficWindowSecs={trafficWindowSecs}
            />
          ))}
        </ul>
      )}
    </section>
  );
};
