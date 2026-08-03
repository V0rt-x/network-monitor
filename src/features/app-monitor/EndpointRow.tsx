import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import { spanOf } from '../../shared/duration';
import type { EndpointView } from '../../shared/ipc';
import { useFigures } from '../../shared/useFigures';
import { healthKey, healthModifier, probeKindKey } from '../dashboard/labels';
import { MetricHelp } from '../help/MetricHelp';
import { FlowPanel } from './FlowPanel';
import { networkName } from './networkName';
import { ageKindKey, livenessKey, probingKey, transportKey } from './labels';
import { PathPanel } from './PathPanel';

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
  /** The row's own identifier, so a selection on the chart can bring it into view. */
  readonly id: string;
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
  id,
  endpoint,
  trafficWindowSecs,
  colour,
  raised,
  dimmed,
  pinned,
  onPin,
  onHover,
}: EndpointRowProps) => {
  const { t } = useTranslation();
  const figures = useFigures();
  // Absent stays absent here too: a span the core did not send is left off the row rather
  // than written as "0 s", which would say the endpoint had just appeared.
  const age = endpoint.age.secs === null ? null : spanOf(endpoint.age.secs);

  const modifiers = [raised ? 'nm-endpoint--raised' : '', dimmed ? 'nm-endpoint--dimmed' : '']
    .filter(Boolean)
    .join(' ');

  return (
    <li
      id={id}
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
        {/* Level one, and the only thing on the row a reader can recognise without knowing
            what a single one of the figures means. It is a label rather than a figure, so it
            does not count against the three, and it goes beside the address because it is
            what the address *is*. Absent where the directory is off, still loading, or
            simply does not know — there is no nearest network to fall back to, and a wrong
            name is a false statement about where someone's traffic went, not a rounding. */}
        {endpoint.network !== null && (
          <span className="nm-endpoint__network">
            {networkName(endpoint.network, t)}
            <MetricHelp topic="network" />
          </span>
        )}
        {/* How long it has been there, which is what tells a new endpoint from one that has
            been carrying the match all along. One figure at level one under a neutral word:
            it is two different facts depending on the transport, and the expander below
            names which — a single label meaning whichever was available would answer the
            question with a number nobody could interpret. */}
        {age !== null && (
          <span className="nm-endpoint__age">
            {t('apps.age.label')} {t(age.key, age.params)}
            <MetricHelp topic="age" />
          </span>
        )}
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
        {/* Promoted out of the expander. It qualifies every figure on the row rather than
            describing how one of them was taken, and a reader who never opens the details
            would otherwise read a tunnel's round trip as the server's. */}
        {endpoint.tunnelled && (
          <span className="nm-badge">
            {t('dashboard.badge.tunnelled')}
            <MetricHelp topic="tunnel" />
          </span>
        )}
      </div>

      {/* Level one: three figures and nothing else, under the names the rest of the
          networking world uses. The plain-language sentence is the ⓘ's job, not the
          label's — a renamed quantity teaches the reader a vocabulary nobody else speaks.

          Absent entirely where no probe will ever fill them in, and absent *silently*. Rust
          draws the line between *not yet* and *never*: a chain still trying kinds keeps its
          dashes, because a figure is coming, while a match server would carry three of them
          for the whole match — and three dashes where the headline figures belong read as a
          broken tool rather than as an honest absence. Nothing on the row explains the gap
          in prose: what stands in its place is on the card already, and the one-line
          disclosure below says why in the reader's own time. */}
      {endpoint.probesMeasureIt && (
        <dl className="nm-endpoint__metrics">
          <div>
            <dt>
              {t('apps.metric.rtt')}
              <MetricHelp topic="rtt" />
            </dt>
            <dd>{figures.ms(endpoint.rttMs)}</dd>
          </div>
          <div>
            <dt>
              {t('apps.metric.jitter')}
              <MetricHelp topic="jitter" />
            </dt>
            <dd>{figures.ms(endpoint.jitterMs)}</dd>
          </div>
          <div>
            <dt>
              {t('apps.metric.loss')}
              <MetricHelp topic="loss" />
            </dt>
            <dd>{figures.pct(endpoint.lossPct)}</dd>
          </div>
        </dl>
      )}

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
              {/* The tunnel is stated at level one now, beside the health, because it
                  qualifies the figures rather than describing how one was taken. */}
              {endpoint.filteringConfirmed && ` · ${t('dashboard.badge.filteringConfirmed')}`}
            </dd>
          </div>
          {/* Which of the two facts the header's figure is. They are not interchangeable:
              a TCP connection has an establishment the operating system dates, and a UDP
              endpoint has none, so what can be said there is how long we have been
              watching. The word is here rather than beside the number because the number
              answers the question either way. */}
          {age !== null && (
            <div>
              <dt>{t(ageKindKey(endpoint.age.kind))}</dt>
              <dd>{t(age.key, age.params)}</dd>
            </div>
          )}
          {/* Level two: what qualifies the name rather than being it. The number is the
              durable identity a reader can search for; the country is where the network was
              *registered*, which for any large provider is routinely nowhere near the
              machine that answered — so it is worded as a registration and never as a
              location. How old the directory is belongs to the directory, not to each row,
              and is stated once in Settings. */}
          {endpoint.network !== null && (
            <div>
              <dt>
                {t('apps.details.network')}
                <MetricHelp topic="network" />
              </dt>
              <dd>
                {t('apps.network.as', { asn: endpoint.network.asn })}
                {endpoint.network.country !== null &&
                  ` · ${t('apps.network.registeredIn', { country: endpoint.network.country })}`}
              </dd>
            </div>
          )}
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
            <dd>{figures.bytes(endpoint.recentBytes)}</dd>
          </div>
          {/* The one genuine round trip to the endpoint that cost no packet: the operating
              system's own estimate for the application's connection. It carries its age,
              because it arrives every few tens of seconds at best and would otherwise read
              as live. */}
          {endpoint.passiveRtt !== null && (
            <div>
              <dt>{t('apps.metric.stackRtt')}</dt>
              <dd>
                {figures.ms(endpoint.passiveRtt.rttMs)}{' '}
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
                {figures.bytesPerSec(endpoint.flow.receivedBytesPerSec)}
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
