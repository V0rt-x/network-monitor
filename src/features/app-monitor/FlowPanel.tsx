import { useTranslation } from 'react-i18next';

import { formatMs, formatPct, formatRate } from '../../shared/format';
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
 * **Nothing here is a ping and the panel says so.** No request is timed against its answer:
 * the operating system reports that datagrams arrived, not what they were replying to. The
 * spread of their arrivals is what a player feels as stutter, and it folds in the server's
 * own send cadence — a server that skips a tick and a network that delays one look
 * identical from here.
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
 * **Reworded, not thinned.** All five figures stay; each is named for the thing the player
 * experiences — how often the server speaks, how evenly it arrives, the worst pause, how far
 * the return traffic has fallen off, and a freeze. The byte rate and the span they are taken
 * over moved down a level, to the row's own expander, because they qualify the figures rather
 * than being ones a player reads.
 */
export const FlowPanel = ({ flow }: FlowPanelProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const updates = flow.updatesPerSec;

  return (
    <section className="nm-flow">
      <header className="nm-panel__header">
        <h4 className="nm-panel__title">{t('apps.passive.heading')}</h4>
        {/* Kept prominent rather than demoted: a freeze is the one thing here a player
            recognises immediately, and it is the strongest evidence the panel can show. */}
        {flow.stallMs !== null && (
          <span className="nm-health nm-health--bad">
            {t('apps.passive.stall', { ms: Math.round(flow.stallMs) })}
            <MetricHelp topic="freeze" />
          </span>
        )}
      </header>

      <p className="nm-panel__note">{t('apps.passive.note')}</p>

      <dl className="nm-endpoint__metrics">
        <div>
          <dt>
            {t('apps.passive.metric.updates')}
            <MetricHelp topic="updates" />
          </dt>
          <dd>
            {updates === null
              ? t('apps.passive.updatesUnknown')
              : t('apps.passive.updatesPerSec', { rate: formatRate(updates, locale) })}
          </dd>
        </div>
        <div>
          <dt>
            {t('apps.passive.metric.smoothness')}
            <MetricHelp topic="smoothness" />
          </dt>
          <dd>{formatMs(flow.arrivalJitterMs, locale)}</dd>
        </div>
        <div>
          <dt>
            {t('apps.passive.metric.worstPause')}
            <MetricHelp topic="worstPause" />
          </dt>
          <dd>{formatMs(flow.arrivalMaxMs, locale)}</dd>
        </div>
        <div>
          <dt>
            {t('apps.passive.metric.dropOff')}
            <MetricHelp topic="dropOff" />
          </dt>
          <dd>{formatPct(flow.receiveShortfallPct, locale)}</dd>
        </div>
      </dl>
    </section>
  );
};
