import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import { formatBytes, formatMs, formatPct } from '../../shared/format';
import type { EndpointView } from '../../shared/ipc';
import { healthKey, healthModifier, probeKindKey } from '../dashboard/labels';
import { MetricHelp } from '../help/MetricHelp';
import { FlowPanel } from './FlowPanel';
import { livenessKey, probingKey, transportKey } from './labels';
import { PathPanel } from './PathPanel';
import { WhyNotYourPing } from './WhyNotYourPing';

/**
 * How the application's own traffic leaves the machine.
 *
 * An adapter name where the machine gave one — "Wi-Fi", "Ethernet", the accelerator's own
 * adapter — because that is the thing a user comparing before and after a VPN can actually
 * check. The address is kept beside it rather than replaced by it: the name is a label, and
 * the address is what the probe was bound to.
 */
const egressLine = (endpoint: EndpointView, t: TFunction): string => {
  if (endpoint.egress === null) return t('apps.egress.unknown');
  if (endpoint.egressInterface === null) return t('apps.egress.via', { address: endpoint.egress });
  return t('apps.egress.viaNamed', {
    address: endpoint.egress,
    interface: endpoint.egressInterface,
  });
};

interface EndpointRowProps {
  readonly endpoint: EndpointView;
  /** Span the byte count covers, so the traffic figure can say what it is a count of. */
  readonly trafficWindowSecs: number;
  /** The colour this endpoint's line is drawn in, so the row can be tied to it. */
  readonly colour: string;
  /** Whether this is the endpoint currently raised on the chart. */
  readonly raised: boolean;
  /** Whether another endpoint is raised, so this one steps back. */
  readonly dimmed: boolean;
  /** Whether the raise is pinned to this endpoint rather than following the cursor. */
  readonly pinned: boolean;
  readonly onPin: () => void;
  readonly onHover: (endpoint: string | null) => void;
}

/**
 * One endpoint of one application, at the depth the reader asked for.
 *
 * **Two levels, not two products.** The same data from Rust either way; what changes is how
 * much of it is on screen before anyone asks. The page used to state everything it knew at
 * once — probe kind, proven filtering, both egress addresses and their adapters, the window a
 * rate is taken over — and the result was that the numbers that matter were indistinguishable
 * from the caveats attached to them.
 *
 * *Level one*, always visible: what it is, one word of state, and three figures — ping (RTT),
 * jitter, loss. **Those are the words the rest of the networking world uses**, and that is
 * deliberate: the audience has met them in every other tool they have opened, and a quantity
 * renamed around the reader teaches a vocabulary nobody else speaks. The plain-language
 * sentence belongs to the ⓘ, which is what level three is for. *Ping* is honest here and only
 * here — this is a round trip we really measured — and it is never applied to the route
 * figure, which belongs to a router short of the endpoint.
 *
 * For an endpoint nothing can measure the three do not appear at all; what stands in for them
 * is the route beside its own traffic, two quantities that are never merged, because their
 * disagreement is the diagnosis.
 *
 * *Level two* is this row's expander, and there is deliberately **no setting**. A mode is a
 * second product to keep consistent and one a user forgets they are in; an expander is a
 * question asked and answered in place. What lives there is everything that qualifies a
 * number rather than being one: which probe produced it, whether filtering was proven, which
 * adapter the traffic and the probe leave by, what span a rate covers, how many bytes came
 * back, and how far the route reached.
 *
 * **An egress conflict does not move.** It is a warning, not a detail: the figure describes a
 * different route from the one the application is taking, and a user who never opens the
 * expander must still be told.
 *
 * *Level three* is the ⓘ on every figure — one or two plain sentences in place, and a way to
 * the bundled help. The audience is a player who knows their game stutters and does not know
 * what jitter is, and a measurement tool that cannot explain itself is asking to be trusted
 * on faith.
 *
 * **The row is also the keyboard path to the chart.** Selecting it raises this endpoint's
 * line and dims the others — the same thing hovering a line does. The colour swatch ties the
 * two together; it names a line and says nothing about health, which the badge states in
 * words.
 */
export const EndpointRow = ({
  endpoint,
  trafficWindowSecs,
  colour,
  raised,
  dimmed,
  pinned,
  onPin,
  onHover,
}: EndpointRowProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  const modifiers = [raised ? 'nm-endpoint--raised' : '', dimmed ? 'nm-endpoint--dimmed' : '']
    .filter(Boolean)
    .join(' ');

  // Nothing our probes can send reaches this endpoint. That is the normal state of a game's
  // match server rather than a fault, and it is the case the whole product exists for — so
  // the row says in as many words why the figure it shows is not the one the game shows.
  const answersNothing =
    endpoint.rttMs === null && (endpoint.path !== null || endpoint.flow !== null);

  return (
    <li
      className={`nm-endpoint ${modifiers}`.trimEnd()}
      onMouseEnter={() => {
        onHover(endpoint.key);
      }}
      onMouseLeave={() => {
        onHover(null);
      }}
    >
      <div className="nm-endpoint__identity">
        <button
          type="button"
          className="nm-endpoint__select"
          aria-pressed={pinned}
          onClick={onPin}
          onFocus={() => {
            onHover(endpoint.key);
          }}
          onBlur={() => {
            onHover(null);
          }}
        >
          <span
            className="nm-endpoint__swatch"
            style={{ backgroundColor: colour }}
            aria-hidden="true"
          />
          <span className="nm-endpoint__address">{endpoint.address}</span>
          <span className="nm-visually-hidden">{t('apps.chart.highlight')}</span>
        </button>
        <span className="nm-endpoint__transport">{t(transportKey(endpoint.transport))}</span>
      </div>

      <div className="nm-endpoint__badges">
        <span className={`nm-health ${healthModifier(endpoint.health)}`}>
          {t(healthKey(endpoint.health))}
        </span>
        {/* Said out loud, with the time left, rather than shown as three dashes that read
            like a failure. Rust decides when it is over — it is the same "absent knowledge
            stays absent" rule that already withholds a figure whose precondition failed. */}
        {endpoint.warmupSecsRemaining !== null && (
          <span className="nm-badge">
            {t('apps.warmup.badge', { seconds: Math.ceil(endpoint.warmupSecsRemaining) })}
          </span>
        )}
        {/* A warning, never a detail: the figure describes a different route from the one
            this application is taking, and a reader who never opens the expander must still
            be told. */}
        {endpoint.egressConflict && (
          <span className="nm-badge nm-badge--warn">{t('apps.badge.egressConflict')}</span>
        )}
        {!endpoint.measurable && (
          <span className="nm-badge nm-badge--warn">{t('dashboard.badge.notMeasurable')}</span>
        )}
      </div>

      {/* Level one: three figures and nothing else, under the names the rest of the
          networking world uses. The plain-language sentence is the ⓘ's job, not the
          label's — a renamed quantity teaches the reader a vocabulary nobody else speaks. */}
      <dl className="nm-endpoint__metrics">
        <div>
          <dt>
            {t('apps.metric.rtt')}
            <MetricHelp topic="rtt" />
          </dt>
          <dd>{formatMs(endpoint.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>
            {t('apps.metric.jitter')}
            <MetricHelp topic="jitter" />
          </dt>
          <dd>{formatMs(endpoint.jitterMs, locale)}</dd>
        </div>
        <div>
          <dt>
            {t('apps.metric.loss')}
            <MetricHelp topic="loss" />
          </dt>
          <dd>{formatPct(endpoint.lossPct, locale)}</dd>
        </div>
      </dl>

      {answersNothing && <WhyNotYourPing />}

      {/* Two columns, never one. The route is a round trip to a router short of the
          endpoint; the flow is the arrival pattern of the traffic itself. Merging them
          into a single figure called "ping" is the lie this product exists not to tell,
          and their disagreement is the whole diagnosis. Either may be absent — an endpoint
          that answers for itself needs no route, and a machine without the tracing setup
          counts no traffic — so the pair lays out with whichever is there. */}
      {(endpoint.path !== null || endpoint.flow !== null) && (
        <div className="nm-endpoint__columns">
          {endpoint.path !== null && <PathPanel path={endpoint.path} />}
          {endpoint.flow !== null && <FlowPanel flow={endpoint.flow} />}
        </div>
      )}

      <details className="nm-endpoint__details">
        <summary>{t('apps.details.summary')}</summary>
        <dl className="nm-endpoint__detail-list">
          <div>
            <dt>
              {t('apps.details.probeKind')}
              <MetricHelp topic="probeKind" />
            </dt>
            <dd>
              {endpoint.probeKind === null
                ? t('apps.details.probeKindNone')
                : t(probeKindKey(endpoint.probeKind))}
              {endpoint.tunnelled && ` · ${t('dashboard.badge.tunnelled')}`}
              {endpoint.filteringConfirmed && ` · ${t('dashboard.badge.filteringConfirmed')}`}
            </dd>
          </div>
          <div>
            <dt>{t('apps.details.use')}</dt>
            <dd>
              {t(livenessKey(endpoint.liveness))} · {t(probingKey(endpoint.probing))}
            </dd>
          </div>
          <div>
            <dt>
              {t('apps.metric.traffic', { seconds: trafficWindowSecs })}
              <MetricHelp topic="traffic" />
            </dt>
            <dd>{formatBytes(endpoint.recentBytes, locale)}</dd>
          </div>
          {/* The one genuine round trip to the endpoint that cost no packet: the operating
              system's own estimate for the application's connection. It carries its age,
              because it arrives every few tens of seconds at best and would otherwise read
              as live. */}
          {endpoint.passiveRtt !== null && (
            <div>
              <dt>{t('apps.metric.stackRtt')}</dt>
              <dd>
                {formatMs(endpoint.passiveRtt.rttMs, locale)}{' '}
                {/* The age is not decoration: this arrives every few tens of seconds at
                    best, so a figure without it would read as current when it is not. */}
                {endpoint.passiveRtt.ageSecs !== null && (
                  <span className="nm-endpoint__age">
                    {t('apps.metric.stackRttAge', {
                      seconds: Math.round(endpoint.passiveRtt.ageSecs),
                    })}
                  </span>
                )}
              </dd>
            </div>
          )}
          {endpoint.flow !== null && (
            <div>
              <dt>{t('apps.details.incoming')}</dt>
              <dd>
                {t('apps.passive.perSecond', {
                  bytes: formatBytes(endpoint.flow.receivedBytesPerSec, locale),
                })}
                {/* The span is what keeps a rate honest — it says what period the figure is
                    a rate over — so when it is absent the clause goes rather than a guess
                    taking its place. */}
                {endpoint.flow.spanSecs !== null && (
                  <> · {t('apps.passive.span', { seconds: Math.round(endpoint.flow.spanSecs) })}</>
                )}
              </dd>
            </div>
          )}
          {endpoint.path !== null && (
            <div>
              <dt>
                {t('apps.details.route')}
                <MetricHelp topic="route" />
              </dt>
              <dd>
                {endpoint.path.hopTtl === null
                  ? t('apps.path.hopUnknown')
                  : t('apps.path.hop', { ttl: endpoint.path.hopTtl })}
                {' · '}
                {t('apps.path.hopsProbed', { count: endpoint.path.hopsProbed })}
              </dd>
            </div>
          )}
          <div>
            <dt>
              {t('apps.details.egress')}
              <MetricHelp topic="egress" />
            </dt>
            <dd>
              {egressLine(endpoint, t)}
              {/* Only where the probe does not follow the application. Naming the same route
                  twice on every row would bury the one case this disclosure exists for. */}
              {endpoint.probeEgress !== null && (
                <>
                  {' '}
                  <span className="nm-endpoint__egress-probe">
                    {endpoint.probeEgressInterface === null
                      ? t('apps.egress.probe', { address: endpoint.probeEgress })
                      : t('apps.egress.probeNamed', {
                          address: endpoint.probeEgress,
                          interface: endpoint.probeEgressInterface,
                        })}
                  </span>
                </>
              )}
            </dd>
          </div>
        </dl>
      </details>
    </li>
  );
};
