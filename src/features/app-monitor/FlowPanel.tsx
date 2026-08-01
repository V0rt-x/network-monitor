import { useTranslation } from 'react-i18next';

import { formatBytes, formatMs, formatPct, formatRate } from '../../shared/format';
import type { FlowView } from '../../shared/ipc';

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
 * holds steady.
 */
export const FlowPanel = ({ flow }: FlowPanelProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const updates = flow.updatesPerSec;

  return (
    <section className="nm-flow">
      <header className="nm-panel__header">
        <h4 className="nm-panel__title">{t('apps.passive.heading')}</h4>
        {flow.stallMs !== null && (
          <span className="nm-health nm-health--bad">
            {t('apps.passive.stall', { ms: Math.round(flow.stallMs) })}
          </span>
        )}
      </header>

      <p className="nm-panel__note">{t('apps.passive.note')}</p>

      <p className="nm-panel__where">
        {updates === null
          ? t('apps.passive.updatesUnknown')
          : t('apps.passive.updates', { rate: formatRate(updates, locale) })}
        {/* The span is what keeps a rate honest — it says what period the figure is a rate
            over — so when it is absent the clause goes rather than a guess taking its
            place. */}
        {flow.spanSecs !== null && (
          <>
            {' · '}
            {t('apps.passive.span', { seconds: Math.round(flow.spanSecs) })}
          </>
        )}
      </p>

      <dl className="nm-endpoint__metrics">
        <div>
          <dt>{t('apps.passive.metric.arrivalJitter')}</dt>
          <dd>{formatMs(flow.arrivalJitterMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.passive.metric.arrivalWorst')}</dt>
          <dd>{formatMs(flow.arrivalMaxMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.passive.metric.shortfall')}</dt>
          <dd>{formatPct(flow.receiveShortfallPct, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.passive.metric.incoming')}</dt>
          <dd>
            {t('apps.passive.perSecond', { bytes: formatBytes(flow.receivedBytesPerSec, locale) })}
          </dd>
        </div>
      </dl>
    </section>
  );
};
