import { useTranslation } from 'react-i18next';

import { formatMs, formatPct } from '../../shared/format';
import type { TargetView } from '../../shared/ipc';
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
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

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
        {target.tunnelled && <span className="nm-badge">{t('dashboard.badge.tunnelled')}</span>}
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

      <dl className="nm-target__metrics">
        <div>
          <dt>{t('dashboard.metric.rtt')}</dt>
          <dd>{formatMs(target.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.jitter')}</dt>
          <dd>{formatMs(target.jitterMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.loss')}</dt>
          <dd>{formatPct(target.lossPct, locale)}</dd>
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
