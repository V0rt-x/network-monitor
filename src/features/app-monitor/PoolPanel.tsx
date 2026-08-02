import { useTranslation } from 'react-i18next';

import { healthKey, healthModifier } from '../dashboard/labels';
import { formatCount, formatMs, formatPct } from '../../shared/format';
import type { PoolView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';

interface PoolPanelProps {
  readonly pool: PoolView | null;
}

/**
 * What a game's own reference targets say about its infrastructure.
 *
 * The evidence behind the two game-server verdicts, shown so the user can check the
 * conclusion rather than take it on trust. It is deliberately a set of counts and not a
 * headline: "four of eight answering" is a fact, and the sentence built on it lives in the
 * verdict banner where it can be qualified.
 *
 * A missing pool is stated rather than hidden. Most titles publish no reference address, so
 * an application with nothing to compare against is the ordinary case — and an absent pool
 * can neither report an outage nor rule one out, which the user has to know before reading
 * a verdict that stops at the route.
 */
export const PoolPanel = ({ pool }: PoolPanelProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  if (pool === null) {
    return (
      <section className="nm-pool nm-panel">
        <header className="nm-panel__header">
          <h4 className="nm-panel__title">
            {t('apps.pool.title')}
            <MetricHelp topic="pool" />
          </h4>
        </header>
        <p className="nm-panel__note">{t('apps.pool.absent')}</p>
      </section>
    );
  }

  return (
    <section className="nm-pool nm-panel">
      <header className="nm-panel__header">
        <h4 className="nm-panel__title">
          {t('apps.pool.title')}
          <MetricHelp topic="pool" />
        </h4>
        <span className={`nm-health ${healthModifier(pool.health)}`}>
          {t(healthKey(pool.health))}
        </span>
      </header>

      <dl className="nm-pool__metrics">
        <div>
          <dt>{t('apps.pool.answering')}</dt>
          <dd>{formatPct(pool.answeringPct, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.pool.rtt')}</dt>
          <dd>{formatMs(pool.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.pool.members')}</dt>
          <dd>
            {t('apps.pool.membership', {
              seeded: formatCount(pool.seeded, locale),
              learned: formatCount(pool.learned, locale),
            })}
          </dd>
        </div>
      </dl>

      {/* The count stays on the page because it changes what the percentage above it means;
          why a silent member proves nothing is the ⓘ's answer, not a paragraph's. */}
      {pool.unproven > 0 && (
        <p className="nm-panel__note">{t('apps.pool.unproven', { count: pool.unproven })}</p>
      )}
    </section>
  );
};
