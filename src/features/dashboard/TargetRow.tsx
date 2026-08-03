import { useTranslation } from 'react-i18next';

import { useFigures } from '../../shared/useFigures';
import type { TargetView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';
import { healthKey, healthModifier, probeKindKey } from './labels';
import { Sparkline } from './Sparkline';

interface TargetRowProps {
  readonly target: TargetView;
}

/**
 * One baseline target: its verdict, its numbers, and every caveat attached to them.
 *
 * The caveats are not decoration. A round-trip time measured through a local tunnel is not
 * a round trip to the server; a figure produced by a TLS hello is not the same quantity as
 * an ICMP echo; and "we proved ICMP is filtered here" is a different claim from "this host
 * is silent". Each of those is a badge rather than a footnote, because a number shown
 * without them would be quietly wrong.
 */
export const TargetRow = ({ target }: TargetRowProps) => {
  const { t } = useTranslation();
  const figures = useFigures();

  const address = target.resolvedAddress ?? target.writtenAddress;

  return (
    <li className="nm-target">
      <div className="nm-target__identity">
        <span className="nm-target__label">{target.label}</span>
        <span className="nm-target__address" title={target.writtenAddress}>
          {address}
        </span>
      </div>

      <div className="nm-target__badges">
        <span className={`nm-health ${healthModifier(target.health)}`}>
          {t(healthKey(target.health))}
        </span>
        {target.probeKind !== null && (
          <span className="nm-badge">{t(probeKindKey(target.probeKind))}</span>
        )}
        {/* The one badge with an ⓘ. It is not a caveat on the row, it is the reason the
            figures beside it were measured a different way. */}
        {target.tunnelled && (
          <span className="nm-badge">
            {t('dashboard.badge.tunnelled')}
            <MetricHelp topic="tunnel" />
          </span>
        )}
        {target.filteringConfirmed && (
          <span className="nm-badge">{t('dashboard.badge.filteringConfirmed')}</span>
        )}
        {target.resolvedAddress === null && (
          <span className="nm-badge nm-badge--warn">{t('dashboard.badge.unresolved')}</span>
        )}
        {target.resolvedAddress !== null && !target.measurable && (
          <span className="nm-badge nm-badge--warn">{t('dashboard.badge.notMeasurable')}</span>
        )}
      </div>

      {/* The same three quantities the applications page shows, under the same names and
          with the same ⓘ. They were the one set of figures on the merged Network page that
          could not explain itself — which the page beside them could. */}
      <dl className="nm-target__metrics">
        <div>
          <dt>
            {t('dashboard.metric.rtt')}
            <MetricHelp topic="rtt" />
          </dt>
          <dd>{figures.ms(target.rttMs)}</dd>
        </div>
        <div>
          <dt>
            {t('dashboard.metric.jitter')}
            <MetricHelp topic="jitter" />
          </dt>
          <dd>{figures.ms(target.jitterMs)}</dd>
        </div>
        <div>
          <dt>
            {t('dashboard.metric.loss')}
            <MetricHelp topic="loss" />
          </dt>
          <dd>{figures.pct(target.lossPct)}</dd>
        </div>
      </dl>

      <Sparkline
        ageSecs={target.seriesAgeSecs}
        rttMs={target.seriesRttMs}
        health={target.health}
        label={t('dashboard.sparklineLabel', { target: target.label })}
      />
    </li>
  );
};
