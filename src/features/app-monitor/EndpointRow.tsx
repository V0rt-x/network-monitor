import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import { formatBytes, formatMs, formatPct } from '../../shared/format';
import type { EndpointView } from '../../shared/ipc';
import { healthKey, healthModifier, probeKindKey } from '../dashboard/labels';
import { FlowPanel } from './FlowPanel';
import { livenessKey, probingKey, transportKey } from './labels';
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
 * One endpoint of one application: its own verdict, its own numbers, its own caveats.
 *
 * Nothing here is rolled up to the application. Within one game some endpoints stay clean
 * while others lose packets or go unreachable — a login service on a CDN, voice on one
 * provider, the match server on another — and a single colour for the application would
 * either report a working game as broken or hide the failure the user came to find.
 *
 * The caveats are not decoration either. A figure produced by a TLS hello is not the same
 * quantity as an ICMP round trip; a figure measured through a local tunnel is not a round
 * trip to the server at all; an endpoint whose probe egresses somewhere other than the
 * application's own flow is not measuring the route the user is asking about. Each is
 * stated beside the number rather than folded into it.
 *
 * The route caveat covers two cases that look the same to the user and are: another
 * monitored application reaching the endpoint by a different interface, and an address a
 * baseline was already probing, whose binding was chosen before this application asked.
 * Either way the single probe cannot be promised to follow this application's route.
 *
 * An endpoint that answers nothing at all — a game's match server, normally — carries two
 * further panels, side by side and never merged: the route to it, and its own traffic. They
 * sit below the endpoint's figures and never replace them. The dashes stay dashes, because
 * nothing measured the server; the route panel names the router its numbers belong to, and
 * the flow panel measures the data the application is actually exchanging. Their
 * disagreement is the diagnosis — a clean route beside ragged arrivals is the server's
 * problem — and a single combined "ping" would destroy it.
 *
 * **The row is the keyboard path to the chart.** Selecting it raises this endpoint's line
 * and dims the others — the same thing hovering a line does, reachable without a mouse. The
 * colour swatch is what ties the two together; it names a line and says nothing about
 * health, which the badge beside it states in words.
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
        {endpoint.probeKind !== null && (
          <span className="nm-badge">{t(probeKindKey(endpoint.probeKind))}</span>
        )}
        {endpoint.liveness === 'idle' && (
          <span className="nm-badge">{t(livenessKey(endpoint.liveness))}</span>
        )}
        {endpoint.probing === 'demoted' && (
          <span className="nm-badge">{t(probingKey(endpoint.probing))}</span>
        )}
        {endpoint.tunnelled && <span className="nm-badge">{t('dashboard.badge.tunnelled')}</span>}
        {endpoint.filteringConfirmed && (
          <span className="nm-badge">{t('dashboard.badge.filteringConfirmed')}</span>
        )}
        {endpoint.egressConflict && (
          <span className="nm-badge nm-badge--warn">{t('apps.badge.egressConflict')}</span>
        )}
        {!endpoint.measurable && (
          <span className="nm-badge nm-badge--warn">{t('dashboard.badge.notMeasurable')}</span>
        )}
      </div>

      <dl className="nm-endpoint__metrics">
        <div>
          <dt>{t('dashboard.metric.rtt')}</dt>
          <dd>{formatMs(endpoint.rttMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.jitter')}</dt>
          <dd>{formatMs(endpoint.jitterMs, locale)}</dd>
        </div>
        <div>
          <dt>{t('dashboard.metric.loss')}</dt>
          <dd>{formatPct(endpoint.lossPct, locale)}</dd>
        </div>
        <div>
          <dt>{t('apps.metric.traffic', { seconds: trafficWindowSecs })}</dt>
          <dd>{formatBytes(endpoint.recentBytes, locale)}</dd>
        </div>
      </dl>

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

      <p className="nm-endpoint__egress">
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
      </p>
    </li>
  );
};
