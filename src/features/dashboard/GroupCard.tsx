import { useTranslation } from 'react-i18next';

import { formatCount, formatMs, formatPct } from '../../shared/format';
import type { GroupView } from '../../shared/ipc';
import { groupHintKey, groupKey, healthKey, healthModifier } from './labels';
import { TargetRow } from './TargetRow';

interface GroupCardProps {
  readonly group: GroupView;
}

/** Which counts are worth showing, and in what order of severity. */
const DISTRIBUTION = [
  { key: 'unreachable', labelKey: 'dashboard.health.unreachable' },
  { key: 'blocked', labelKey: 'dashboard.health.blocked' },
  { key: 'degraded', labelKey: 'dashboard.health.degraded' },
  { key: 'carryingTraffic', labelKey: 'dashboard.health.carryingTraffic' },
  { key: 'ok', labelKey: 'dashboard.health.ok' },
  { key: 'unknown', labelKey: 'dashboard.health.unknown' },
] as const;

/**
 * One baseline — domestic or foreign — with its headline verdict and the distribution
 * behind it.
 *
 * The distribution is shown whenever the members disagree, and that is the point of the
 * card: "3 clean, 1 unreachable" is actionable, one amber dot is not. A verdict alone
 * would either hide a failure or report a working baseline as broken, and both are the
 * kind of lie this product exists to avoid.
 */
export const GroupCard = ({ group }: GroupCardProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  const counts = DISTRIBUTION.map((entry) => ({
    ...entry,
    value: group.counts[entry.key],
  })).filter((entry) => entry.value > 0);

  return (
    <section className="nm-group">
      <header className="nm-group__header">
        <div>
          <h3 className="nm-group__title">{t(groupKey(group.group))}</h3>
          <p className="nm-group__hint">{t(groupHintKey(group.group))}</p>
        </div>
        <span className={`nm-health nm-health--lg ${healthModifier(group.verdict)}`}>
          {t(healthKey(group.verdict))}
        </span>
      </header>

      <dl className="nm-group__metrics">
        <div>
          <dt>{t('dashboard.metric.rttMedian')}</dt>
          <dd>{formatMs(group.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.jitterMedian')}</dt>
          <dd>{formatMs(group.jitterMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.loss')}</dt>
          <dd>{formatPct(group.lossPct, locale)}</dd>
        </div>
      </dl>

      {counts.length > 0 && (
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

      {group.targets.length === 0 ? (
        <p className="nm-state--pending">{t('dashboard.noTargets')}</p>
      ) : (
        <ul className="nm-group__targets">
          {group.targets.map((target) => (
            <TargetRow key={target.key} target={target} />
          ))}
        </ul>
      )}
    </section>
  );
};
