import { useTranslation } from 'react-i18next';

import { formatMs, formatPct } from '../../shared/format';
import type { PathView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';
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
 * **The two are never merged into one number called "ping", and this panel never uses the
 * word at all.** The distance from that router to the server is unknown: the server answered
 * at no time-to-live at all, so it may be one hop beyond or ten. Elsewhere on the page a
 * measured round trip is labelled *Ping (RTT)*, because that is the word the audience knows
 * and there it is true; here it would be a claim about a server we never reached, so the
 * figure is named for what it is a round trip *to*. A test pins that the word never appears.
 * The panel always says which hop the figures belong to and where that hop sits, and the note
 * above them says what they are not.
 *
 * The verdict has a state the user has to be able to tell from a fault: routers rate-limit
 * echoes addressed to themselves while forwarding traffic perfectly, so a figure that moved
 * at the last hop alone is reported as ambiguous rather than as a degraded path.
 *
 * **It leads with one number**, under a heading that says what that number is a round trip
 * *to*. How many hops are being watched and where the route stops are a level down, in the
 * row's own expander: they qualify the figure rather than being what a player reads. What
 * does not move is the sentence saying this is not the round trip to the server — that is the
 * rule this panel exists to protect.
 */
export const PathPanel = ({ path }: PathPanelProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  return (
    <section className="nm-path">
      <header className="nm-panel__header">
        <h4 className="nm-panel__title">{t('apps.path.heading')}</h4>
        <span className={`nm-health ${pathQualityModifier(path.quality)}`}>
          {t(pathQualityKey(path.quality))}
        </span>
      </header>

      <p className="nm-panel__note">{t('apps.path.note')}</p>

      <dl className="nm-endpoint__metrics">
        <div>
          <dt>
            {t('apps.path.metric.rtt')}
            <MetricHelp topic="route" />
          </dt>
          <dd>{formatMs(path.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.metric.jitter')}</dt>
          <dd>{formatMs(path.jitterMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.metric.loss')}</dt>
          <dd>{formatPct(path.lossPct, locale)}</dd>
        </div>
      </dl>

      <p className="nm-panel__where">{t(pathPositionKey(path.position))}</p>
    </section>
  );
};
