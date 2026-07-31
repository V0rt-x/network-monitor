import { useTranslation } from 'react-i18next';

import { formatMs, formatPct } from '../../shared/format';
import type { PathView } from '../../shared/ipc';
import { pathPositionKey, pathQualityKey, pathQualityModifier } from './labels';

interface PathPanelProps {
  readonly path: PathView;
}

/**
 * The route to an endpoint that answers nothing, shown as its own quantity.
 *
 * A game's match server replies to no echo, no handshake and no hello — nothing listens on a
 * game port but the game — so the endpoint's own figures beside this panel are dashes, and
 * they stay dashes. What can be measured is the deepest router that does answer on the way
 * there, and that is what this shows.
 *
 * **The two are never merged into one number called "ping".** The distance from that router
 * to the server is unknown: the server answered at no time-to-live at all, so it may be one
 * hop beyond or ten. The panel therefore always says which hop the figures belong to and
 * where that hop sits, and the note above them says what they are not.
 *
 * The verdict has a state the user has to be able to tell from a fault: routers rate-limit
 * echoes addressed to themselves while forwarding traffic perfectly, so a figure that moved
 * at the last hop alone is reported as ambiguous rather than as a degraded path.
 */
export const PathPanel = ({ path }: PathPanelProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  return (
    <section className="nm-path">
      <header className="nm-path__header">
        <h4 className="nm-path__title">{t('apps.path.heading')}</h4>
        <span className={`nm-health ${pathQualityModifier(path.quality)}`}>
          {t(pathQualityKey(path.quality))}
        </span>
      </header>

      <p className="nm-path__note">{t('apps.path.note')}</p>

      <p className="nm-path__where">
        {path.hopTtl === null
          ? t('apps.path.hopUnknown')
          : t('apps.path.hop', { ttl: path.hopTtl })}
        {' · '}
        {t('apps.path.hopsProbed', { count: path.hopsProbed })}
        {' · '}
        {t(pathPositionKey(path.position))}
      </p>

      <dl className="nm-endpoint__metrics">
        <div>
          <dt>{t('dashboard.metric.rtt')}</dt>
          <dd>{formatMs(path.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.jitter')}</dt>
          <dd>{formatMs(path.jitterMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.loss')}</dt>
          <dd>{formatPct(path.lossPct, locale)}</dd>
        </div>
      </dl>
    </section>
  );
};
