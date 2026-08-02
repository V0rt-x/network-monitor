import { useTranslation } from 'react-i18next';

import type { EndpointGroupView, FlowStatusView } from '../../shared/ipc';
import { Distribution } from './Distribution';
import { EndpointRow } from './EndpointRow';
import { groupHintKey, groupKey } from './labels';

interface EndpointGroupProps {
  readonly group: EndpointGroupView;
  /** Whether per-process flow events are running, which decides why a UDP group is empty. */
  readonly flowStatus: FlowStatusView;
  /** Span the byte counts cover. */
  readonly trafficWindowSecs: number;
  /** The colour an endpoint's line is drawn in, so a row can be tied to it. */
  readonly colourOf: (endpoint: string) => string;
  /** The endpoint currently raised on the chart, by hover or by pinning. */
  readonly highlighted: string | null;
  readonly pinned: string | null;
  readonly onPin: (endpoint: string) => void;
  readonly onHover: (endpoint: string | null) => void;
}

/**
 * One transport's worth of an application's endpoints: the match traffic, or the supporting
 * connections.
 *
 * **The match traffic comes first because that is where the game is played.** Ordered by
 * severity alone the UDP flows sit wherever their health happens to put them — between a
 * launcher's connection, a content network and a telemetry host — and the endpoint the user
 * came to look at is the one they have to hunt for. The severity ordering is untouched; it
 * now applies inside each group, and it still comes from Rust.
 *
 * **The supporting connections are demoted, never hidden.** A login service or a content
 * network with a filter on it is exactly what "I cannot get into the game" looks like, so
 * the group carries its own distribution in the header and unfolds itself whenever any
 * member is worse than clean. Rust decides that (`needsAttention`), because what a user must
 * not miss is a judgement and every judgement in this product lives where it is tested.
 *
 * **An empty match-traffic group has to say why.** Without the one-time tracing setup there
 * are no UDP endpoints at all on a Windows machine, and an unexplained empty group reads as
 * a game that plays over nothing.
 */
export const EndpointGroup = ({
  group,
  flowStatus,
  trafficWindowSecs,
  colourOf,
  highlighted,
  pinned,
  onPin,
  onHover,
}: EndpointGroupProps) => {
  const { t } = useTranslation();

  const isMatchTraffic = group.transport === 'udp';
  const rows = (
    <ul className="nm-appcard__endpoints">
      {group.endpoints.map((endpoint) => (
        <EndpointRow
          key={endpoint.key}
          endpoint={endpoint}
          trafficWindowSecs={trafficWindowSecs}
          colour={colourOf(endpoint.key)}
          raised={highlighted === endpoint.key}
          dimmed={highlighted !== null && highlighted !== endpoint.key}
          pinned={pinned === endpoint.key}
          onPin={() => {
            onPin(endpoint.key);
          }}
          onHover={onHover}
        />
      ))}
    </ul>
  );

  const heading = (
    <>
      <span className="nm-endpointgroup__title">{t(groupKey(group.transport))}</span>
      <span className="nm-endpointgroup__hint">{t(groupHintKey(group.transport))}</span>
      <Distribution
        counts={group.counts}
        label={t('apps.distribution.group', { group: t(groupKey(group.transport)) })}
        className="nm-distribution--inline"
      />
    </>
  );

  // The match traffic is never folded away; the supporting connections may start folded, and
  // only when every one of them is clean.
  if (isMatchTraffic) {
    return (
      <section className="nm-endpointgroup">
        <header className="nm-endpointgroup__header">{heading}</header>
        {group.endpoints.length === 0 ? (
          <p className="nm-state--pending">
            {flowStatus === 'active' ? t('apps.group.udpEmpty') : t('apps.group.udpUnobservable')}
          </p>
        ) : (
          rows
        )}
      </section>
    );
  }

  if (group.endpoints.length === 0) return null;

  return (
    <details className="nm-endpointgroup" open={group.needsAttention}>
      <summary className="nm-endpointgroup__header">{heading}</summary>
      {rows}
    </details>
  );
};
