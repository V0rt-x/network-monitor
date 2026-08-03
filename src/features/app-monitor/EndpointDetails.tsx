import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import { spanOf } from '../../shared/duration';
import type { EndpointView } from '../../shared/ipc';
import { useFigures } from '../../shared/useFigures';
import { healthKey, probeKindKey } from '../dashboard/labels';
import { MetricHelp } from '../help/MetricHelp';
import { FlowPanel } from './FlowPanel';
import { ageKindKey, livenessKey, probingKey } from './labels';
import { PathPanel } from './PathPanel';
import { useQualifiers } from './qualifiers';

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

interface EndpointDetailsProps {
  readonly endpoint: EndpointView;
  /** Span the byte count covers, so the traffic figure can say what it is a count of. */
  readonly trafficWindowSecs: number;
  /**
   * Whether the route and the traffic panels belong here.
   *
   * They are level one for the busiest flow — which already shows them above — and level two
   * for every other endpoint, whose row is a table row with no room for seven figures.
   */
  readonly withPanels: boolean;
}

/**
 * Level two: everything that *qualifies* a figure rather than being one.
 *
 * Shared by the busiest flow's own expander and by every table row's, so the two can never
 * disagree about what a caveat is. There is deliberately **no setting** controlling any of
 * it: a mode is a second product to keep consistent and one a user forgets they are in,
 * while an expander is a question asked and answered in place.
 *
 * What lives here is which probe produced a figure, whether filtering was proven, which
 * adapter the traffic and the probe leave by, what span a rate covers, how many bytes came
 * back, how far the route reached, and — for every endpoint except the busiest — the route
 * and traffic panels in full.
 *
 * **A warning never lands here.** An egress conflict, a freeze, a proven block stay at level
 * one on the row itself, because the test is whether there is something to do about it.
 */
export const EndpointDetails = ({
  endpoint,
  trafficWindowSecs,
  withPanels,
}: EndpointDetailsProps) => {
  const { t } = useTranslation();
  const figures = useFigures();
  const qualifiers = useQualifiers(endpoint);
  // Absent stays absent here too: a span the core did not send is left off rather than
  // written as "0 s", which would say the endpoint had just appeared.
  const age = endpoint.age.secs === null ? null : spanOf(endpoint.age.secs);

  return (
    <div className="nm-endpoint__details">
      {/* Two panels, never one merged figure. The route is a round trip to a router short of
          the endpoint; the flow is the arrival pattern of the traffic itself. Merging them
          into a single number called "ping" is the lie this product exists not to tell, and
          their disagreement is the whole diagnosis. */}
      {withPanels && (endpoint.path !== null || endpoint.flow !== null) && (
        <div className="nm-endpoint__columns">
          {endpoint.path !== null && <PathPanel path={endpoint.path} />}
          {endpoint.flow !== null && <FlowPanel flow={endpoint.flow} />}
        </div>
      )}

      <dl className="nm-endpoint__detail-list">
        {/* The state in words, which is the third place it is reachable from — beside the
            token's accessible name and the words it shows on hover and on focus. The token
            says which of six states it is at a glance; this says it without one. */}
        <div>
          <dt>{t('apps.details.state')}</dt>
          <dd>
            {[t(healthKey(endpoint.health)), ...qualifiers.map((qualifier) => qualifier.name)].join(
              ' · ',
            )}
          </dd>
        </div>
        <div>
          <dt>
            <MetricHelp topic="probeKind">{t('apps.details.probeKind')}</MetricHelp>
          </dt>
          <dd>
            {endpoint.probeKind === null
              ? t('apps.details.probeKindNone')
              : t(probeKindKey(endpoint.probeKind))}
            {/* The tunnel is stated at level one, beside the health, because it qualifies
                the figures rather than describing how one was taken. */}
            {endpoint.filteringConfirmed && ` · ${t('dashboard.badge.filteringConfirmed')}`}
          </dd>
        </div>
        {/* How long it has been there, and which of the two facts that is. They are not
            interchangeable: a TCP connection has an establishment the operating system
            dates, and a UDP endpoint has none, so what can be said there is how long this
            application has been watched talking to it. */}
        {age !== null && (
          <div>
            <dt>
              <MetricHelp topic="age">{t(ageKindKey(endpoint.age.kind))}</MetricHelp>
            </dt>
            <dd>{t(age.key, age.params)}</dd>
          </div>
        )}
        {/* What qualifies the network's name rather than being it. The number is the durable
            identity a reader can search for; the country is where the network was
            *registered*, which for any large provider is routinely nowhere near the machine
            that answered — so it is worded as a registration and never as a location. */}
        {endpoint.network !== null && (
          <div>
            <dt>
              <MetricHelp topic="network">{t('apps.details.network')}</MetricHelp>
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
            <MetricHelp topic="traffic">
              {t('apps.metric.traffic', { seconds: trafficWindowSecs })}
            </MetricHelp>
          </dt>
          <dd>{figures.bytes(endpoint.recentBytes)}</dd>
        </div>
        {/* The one genuine round trip to the endpoint that cost no packet: the operating
            system's own estimate for the application's connection. */}
        {endpoint.passiveRtt !== null && (
          <div>
            <dt>{t('apps.metric.stackRtt')}</dt>
            <dd>
              {figures.ms(endpoint.passiveRtt.rttMs)}{' '}
              {/* The age is not decoration: this arrives every few tens of seconds at best,
                  so a figure without it would read as current when it is not. */}
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
              {/* The span is what keeps a rate honest — it says what period the figure is a
                  rate over — so when it is absent the clause goes rather than a guess taking
                  its place. */}
              {endpoint.flow.spanSecs !== null && (
                <> · {t('apps.passive.span', { seconds: Math.round(endpoint.flow.spanSecs) })}</>
              )}
            </dd>
          </div>
        )}
        {endpoint.path !== null && (
          <div>
            <dt>
              <MetricHelp topic="route">{t('apps.details.route')}</MetricHelp>
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
            <MetricHelp topic="egress">{t('apps.details.egress')}</MetricHelp>
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
    </div>
  );
};
