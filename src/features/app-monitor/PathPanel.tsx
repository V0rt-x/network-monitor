import { useTranslation } from 'react-i18next';

import { useFigures } from '../../shared/useFigures';
import type { PathView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';
import { networkName } from './networkName';
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
 * **It leads with one number, and the claim lives in that number's name.** The heading says
 * what the panel is about and the label says what the figure is a round trip *to* — a router
 * on the way, not the server. That is where the Phase 5 protection sits now: in the naming,
 * which a reader cannot skip, rather than in a paragraph underneath, which they can. The
 * paragraph is gone under the standing rule that the everyday page carries figures and
 * findings and never explanations; the same words are one keystroke away in the ⓘ and in the
 * help, and a test pins that the word "ping" appears nowhere in this panel.
 *
 * How many hops are being watched and where the route stops are a level down, in the row's
 * own expander: they qualify the figure rather than being what a player reads.
 */
export const PathPanel = ({ path }: PathPanelProps) => {
  const { t } = useTranslation();
  const figures = useFigures();

  return (
    <section className="nm-path">
      <header className="nm-panel__header">
        <h4 className="nm-panel__title">
          <MetricHelp topic="route">{t('apps.path.heading')}</MetricHelp>
        </h4>
        <span className={`nm-health ${pathQualityModifier(path.quality)}`}>
          {t(pathQualityKey(path.quality))}
        </span>
      </header>

      {/* The panel's one number needs no ⓘ of its own — the heading's is about exactly this
          figure, and two disclosures with the same words would be a second thing to read
          rather than a second thing to learn. The jitter and the loss beside it are the same
          quantities the rest of the app names, and they carry the same explanations. */}
      <dl className="nm-endpoint__metrics">
        <div>
          <dt>{t('apps.path.metric.rtt')}</dt>
          <dd>{figures.ms(path.rttMs)}</dd>
        </div>
        <div>
          <dt>
            <MetricHelp topic="jitter">{t('apps.metric.jitter')}</MetricHelp>
          </dt>
          <dd>{figures.ms(path.jitterMs)}</dd>
        </div>
        <div>
          <dt>
            <MetricHelp topic="loss">{t('apps.metric.loss')}</MetricHelp>
          </dt>
          <dd>{figures.pct(path.lossPct)}</dd>
        </div>
      </dl>

      {/* Where the route stops, and — the half that turns it into a diagnosis — whose
          equipment it stopped on. "Inside your provider's network" and "at a border transit
          network" produce identical figures and mean opposite things, and until this label
          existed the panel could only tell them apart by position. A name, not a sentence
          about one: the finding belongs on the page, the explanation belongs in the ⓘ. */}
      <p className="nm-panel__where">
        {t(pathPositionKey(path.position))}
        {path.hopNetwork !== null && (
          <>
            {' · '}
            <MetricHelp topic="network">{networkName(path.hopNetwork, t)}</MetricHelp>
          </>
        )}
      </p>
    </section>
  );
};
