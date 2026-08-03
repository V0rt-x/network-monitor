import { useTranslation } from 'react-i18next';

import { MetricRow } from '../../shared/MetricRow';
import { useFigures } from '../../shared/useFigures';
import type { GroupView } from '../../shared/ipc';
import { Distribution } from '../app-monitor/Distribution';
import { MetricHelp } from '../help/MetricHelp';
import { groupHintKey, groupKey, healthKey, healthModifier } from './labels';
import { TargetRow } from './TargetRow';

interface GroupCardProps {
  readonly group: GroupView;
}

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
  const { t } = useTranslation();
  const figures = useFigures();

  return (
    <section className="nm-group">
      <header className="nm-group__header">
        <div className="nm-group__identity">
          <h3 className="nm-group__title">{t(groupKey(group.group))}</h3>
          <p className="nm-group__hint">{t(groupHintKey(group.group))}</p>
        </div>
        <span className={`nm-health nm-health--lg ${healthModifier(group.verdict)}`}>
          {t(healthKey(group.verdict))}
        </span>
      </header>

      {/* Medians, and they say so: a group figure is not one measurement, and one member on
          a bad path must not drag the headline away from what everyone else sees. */}
      <MetricRow
        size="headline"
        metrics={[
          {
            key: 'rtt',
            label: <MetricHelp topic="medianRtt">{t('dashboard.metric.rttMedian')}</MetricHelp>,
            value: figures.ms(group.rttMs),
          },
          {
            key: 'jitter',
            label: <MetricHelp topic="jitter">{t('dashboard.metric.jitterMedian')}</MetricHelp>,
            value: figures.ms(group.jitterMs),
          },
          {
            key: 'loss',
            label: <MetricHelp topic="loss">{t('dashboard.metric.loss')}</MetricHelp>,
            value: figures.pct(group.lossPct),
          },
        ]}
      />

      {/* One component for the count chips across the whole product: the same object was
          drawn by three components in two visual languages. */}
      <Distribution
        counts={group.counts}
        label={t('dashboard.distribution', { group: t(groupKey(group.group)) })}
      />

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
