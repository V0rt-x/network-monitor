import { useTranslation } from 'react-i18next';

import { MetricHelp } from '../help/MetricHelp';
import type { ServiceGroup, ServiceView } from '../../shared/ipc';
import {
  CHECK_MARKS,
  checkMarkKey,
  checkModifier,
  serviceGroupHintKey,
  serviceGroupKey,
} from './labels';
import { ServiceCard } from './ServiceCard';
import { useServiceStatus } from './useServiceStatus';

/** The shelves, in the order the page shows them. */
const GROUPS = ['gamingPlatform', 'infrastructure'] as const satisfies readonly ServiceGroup[];

/**
 * Live reachability of the platforms and the infrastructure a player depends on.
 *
 * The page answers *is it them or me*, and it is careful about which. Every card reports
 * whether **this machine** can reach an operator's published front door; it never claims a
 * company's service is down, because from inside a filtered network those two look
 * identical and only one of them is something the app can actually observe.
 *
 * Grouped rather than listed flat, because the two shelves fail differently: one storefront
 * being unreachable is that storefront's problem, while three clouds going quiet at once is
 * the user's route out of the country.
 */
export const StatusPage = () => {
  const { t } = useTranslation();
  const status = useServiceStatus();

  if (status.kind === 'waiting') {
    return <p className="nm-state--pending">{t('status.waiting')}</p>;
  }

  if (status.kind === 'unavailable') {
    return (
      <p className="nm-state--degraded" role="alert">
        {t('status.unavailable')}
      </p>
    );
  }

  const { services, checkIntervalSecs, windowSecs, timelinePoints } = status.status;
  const inGroup = (group: ServiceGroup): ServiceView[] =>
    services.filter((service) => service.group === group);

  return (
    <div className="nm-status">
      <p className="nm-status__cadence">{t('status.cadence', { seconds: checkIntervalSecs })}</p>
      <p className="nm-status__caveat">{t('status.caveat')}</p>

      {/* Once for the page, not once per card. The strip is the same strip on every card,
          and the three facts a reader needs about it — one cell is one check, oldest is on
          the left, and this is how far back a full one reaches — were previously nowhere
          but in the source. The legend is also what makes colour stop being the only
          channel: every state is named beside the colour that carries it. */}
      <section className="nm-status__legend">
        <h2 className="nm-status__legendtitle">
          <MetricHelp topic="checks">{t('status.timeline.heading')}</MetricHelp>
        </h2>
        <p className="nm-status__legendnote">
          {t('status.timeline.note', {
            count: timelinePoints,
            minutes: Math.max(1, Math.round(windowSecs / 60)),
          })}
        </p>
        <ul className="nm-status__marks" aria-label={t('status.timeline.legendLabel')}>
          {CHECK_MARKS.map((mark) => (
            <li key={mark} className="nm-status__mark">
              <span className={`nm-check ${checkModifier(mark)}`} aria-hidden="true" />
              {t(checkMarkKey(mark))}
            </li>
          ))}
        </ul>
      </section>

      {GROUPS.map((group) => {
        const members = inGroup(group);
        return (
          <section key={group} className="nm-status__shelf">
            <header className="nm-status__shelfhead">
              <h2 className="nm-status__shelftitle">{t(serviceGroupKey(group))}</h2>
              <p className="nm-status__shelfhint">{t(serviceGroupHintKey(group))}</p>
            </header>

            {members.length === 0 ? (
              <p className="nm-state--pending">{t('status.noServices')}</p>
            ) : (
              <div className="nm-status__cards">
                {members.map((service) => (
                  <ServiceCard
                    key={service.id}
                    service={service}
                    checkIntervalSecs={checkIntervalSecs}
                  />
                ))}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
};
