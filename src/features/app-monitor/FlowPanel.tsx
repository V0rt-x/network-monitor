import { useTranslation } from 'react-i18next';

import { useFigures } from '../../shared/useFigures';
import type { FlowView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';

interface FlowPanelProps {
  readonly flow: FlowView;
}

/**
 * What an endpoint's own traffic says, measured without sending anything.
 *
 * The second of the two columns a silent endpoint gets, and a different quantity from the
 * first. {@link PathPanel} shows a round trip to a router short of the endpoint; this shows
 * the arrival pattern of the data the application is actually exchanging — counted by the
 * operating system, at no cost in packets, and the only figure here that describes the
 * user's real traffic rather than a substitute for it.
 *
 * **Nothing here is a ping, and the figures' own names are what say so.** No request is timed
 * against its answer: the operating system reports that datagrams arrived, not what they were
 * replying to. Not one label here is a round trip, which is the honest way to make that point
 * on a page that carries no explanations; the sentence that used to make it lives in the ⓘ on
 * this panel's heading. The spread of arrivals is what a player feels as stutter, and it folds
 * in the server's own send cadence — a server that skips a tick and a network that delays one
 * look identical from here.
 *
 * That is exactly why the two columns sit side by side rather than being merged. Their
 * disagreement is the diagnosis: a clean route beside ragged arrivals is the server's
 * problem, a ragged route beside clean arrivals is a router rate-limiting the probes we
 * address to it. One combined number would destroy the only reading available for an
 * endpoint nothing can be sent to.
 *
 * The shortfall figure is deliberately not called loss. Only the far end knows what it
 * sent, so a datagram that never arrived is invisible from here; what can be said is that
 * less is coming back than this endpoint's own recent past established, while what we send
 * holds steady. It keeps that careful name in every language.
 *
 * **Reworded, not thinned.** All five figures stay. The one with a standard network term
 * behind it carries that term — the spread of arrivals is *arrival jitter*, and the qualifier
 * is not decoration: the probe's own jitter can be read on the same card, and two figures
 * called "jitter" would be a worse failure than an invented word. The rest name quantities
 * that have no standard term to return to — how often the server speaks, the worst pause, how
 * far the return traffic has fallen off, and a freeze — so they stay named for what the player
 * experiences. The byte rate and the span they are taken over moved down a level, to the row's
 * own expander, because they qualify the figures rather than being ones a player reads.
 */
export const FlowPanel = ({ flow }: FlowPanelProps) => {
  const { t } = useTranslation();
  const figures = useFigures();
  const updates = flow.updatesPerSec;

  return (
    <section className="nm-flow">
      <header className="nm-panel__header">
        {/* The ⓘ moved onto the heading with the paragraph it replaces: that nothing here is
            a ping, and why these figures sit beside the route rather than instead of it, is
            a question about the whole panel rather than about any one figure in it. */}
        <h4 className="nm-panel__title">
          <MetricHelp topic="passive">{t('apps.passive.heading')}</MetricHelp>
        </h4>
        {/* Kept prominent rather than demoted: a freeze is the one thing here a player
            recognises immediately, and it is the strongest evidence the panel can show. */}
        {/* `nm-health--unreachable`, the same red every other "nothing is getting through"
            state on the page uses. It carried a `--bad` modifier no stylesheet defined, so
            the strongest evidence in the product rendered as a neutral pill. */}
        {flow.stallMs !== null && (
          <span className="nm-health nm-health--unreachable">
            <MetricHelp topic="freeze">
              {t('apps.passive.stall', { ms: Math.round(flow.stallMs) })}
            </MetricHelp>
          </span>
        )}
      </header>

      <dl className="nm-endpoint__metrics">
        <div>
          <dt>
            <MetricHelp topic="updates">{t('apps.passive.metric.updates')}</MetricHelp>
          </dt>
          <dd>
            {updates === null
              ? t('apps.passive.updatesUnknown')
              : t('apps.passive.updatesPerSec', { rate: figures.rate(updates) })}
          </dd>
        </div>
        <div>
          <dt>
            <MetricHelp topic="arrivalJitter">{t('apps.passive.metric.arrivalJitter')}</MetricHelp>
          </dt>
          <dd>{figures.ms(flow.arrivalJitterMs)}</dd>
        </div>
        <div>
          <dt>
            <MetricHelp topic="worstPause">{t('apps.passive.metric.worstPause')}</MetricHelp>
          </dt>
          <dd>{figures.ms(flow.arrivalMaxMs)}</dd>
        </div>
        <div>
          <dt>
            <MetricHelp topic="dropOff">{t('apps.passive.metric.dropOff')}</MetricHelp>
          </dt>
          <dd>{figures.pct(flow.receiveShortfallPct)}</dd>
        </div>
      </dl>
    </section>
  );
};
