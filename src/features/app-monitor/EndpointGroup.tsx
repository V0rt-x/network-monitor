import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { EndpointGroupView, FlowStatusView } from '../../shared/ipc';
import { MetricHelp } from '../help/MetricHelp';
import { rowIdOf } from './chartSeries';
import { Distribution } from './Distribution';
import type { SwatchShape } from './endpointColours';
import { EndpointRow } from './EndpointRow';
import { useHeldOrder } from './heldOrder';
import { holdPlace } from './holdPlace';
import { groupKey, groupTopic } from './labels';

/** Endpoint, network, state, ping, jitter, loss, route, and the disclosure. */
const COLUMNS = 8;

interface EndpointGroupProps {
  /** Which application these rows belong to, so a chart selection can find one. */
  readonly appId: number;
  readonly group: EndpointGroupView;
  /** Whether per-process flow events are running, which decides why a UDP group is empty. */
  readonly flowStatus: FlowStatusView;
  /** Span the byte counts cover. */
  readonly trafficWindowSecs: number;
  /** The colour an endpoint's line is drawn in, so a row can be tied to it. */
  readonly colourOf: (endpoint: string) => string;
  /** And its shape, so that pairing is not carried by colour alone. */
  readonly shapeOf: (endpoint: string) => SwatchShape;
  /** The endpoint currently raised on the chart, by hover or by pinning. */
  readonly highlighted: string | null;
  readonly pinned: string | null;
  /** Where the pinned endpoint sat when it was pinned, so it can be held there. */
  readonly pinnedAt: number | null;
  readonly onPin: (endpoint: string, index: number) => void;
  readonly onHover: (endpoint: string | null) => void;
}

/**
 * One transport's worth of an application's endpoints, as a table.
 *
 * **UDP comes first, and the heading claims nothing beyond the transport.** During a game the
 * endpoints that decide whether it plays well are usually the UDP flows, and severity alone
 * puts them wherever their health happens to fall — between a launcher's connection, a
 * content network and a telemetry host. So they lead. What the heading must *not* do is turn
 * that tendency into a claim about purpose, which is why it says `UDP flows` and the sentence
 * that used to sit beside it is now what its explanation says.
 *
 * **The TCP connections are demoted, never hidden.** A login service or a content network
 * with a filter on it is exactly what "I cannot get into the game" looks like, so the group
 * carries its own distribution in the header — visible folded or not — and starts open
 * whenever any member is worse than clean at the moment the card first draws. After that the
 * fold belongs to the reader: it used to re-apply that decision on every render, and on a
 * weak link the value flips constantly, so the section opened and collapsed under them.
 *
 * **An empty UDP group has to say why.** Without the one-time tracing setup there are no UDP
 * endpoints at all on a Windows machine, and an unexplained empty group reads as a game that
 * plays over nothing.
 *
 * **The explanations live in the headings, once each.** That is the whole reason this is a
 * table: a column heading exists once however many rows there are, so an application with
 * twenty endpoints carries exactly as many disclosures as one with three.
 */
export const EndpointGroup = ({
  appId,
  group,
  flowStatus,
  trafficWindowSecs,
  colourOf,
  shapeOf,
  highlighted,
  pinned,
  pinnedAt,
  onPin,
  onHover,
}: EndpointGroupProps) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(group.needsAttention);

  const isMatchTraffic = group.transport === 'udp';
  // Rust's severity order, held still while the list is being read. Every state change
  // anywhere re-sorts it, so without this the row under the reader's pointer swaps with its
  // neighbour while they are looking at it.
  const { shown: settled, holdProps } = useHeldOrder(group.endpoints);
  // And the reader's own pin on top of that, which survives leaving the list.
  const shown = holdPlace(
    settled,
    settled.find((endpoint) => endpoint.key === pinned) ?? null,
    pinnedAt,
  );

  const rows = (
    <table className="nm-endpoints" {...holdProps}>
      <thead>
        <tr>
          <th scope="col">{t('apps.column.endpoint')}</th>
          <th scope="col">
            <MetricHelp topic="network">{t('apps.column.network')}</MetricHelp>
          </th>
          <th scope="col">{t('apps.column.state')}</th>
          <th scope="col">
            <MetricHelp topic="rtt">{t('apps.metric.rtt')}</MetricHelp>
          </th>
          <th scope="col">
            <MetricHelp topic="jitter">{t('apps.metric.jitter')}</MetricHelp>
          </th>
          <th scope="col">
            <MetricHelp topic="loss">{t('apps.metric.loss')}</MetricHelp>
          </th>
          {/* Named for what it is a round trip *to* — a router on the way, not the server.
              A blank `Ping` beside a filled `Route` is what makes the never-merge rule
              visible on every row at once. */}
          <th scope="col">
            <MetricHelp topic="route">{t('apps.column.route')}</MetricHelp>
          </th>
          <th scope="col">
            <span className="nm-visually-hidden">{t('apps.column.details')}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {shown.map((endpoint, index) => (
          <EndpointRow
            key={endpoint.key}
            id={rowIdOf(appId, endpoint.key)}
            endpoint={endpoint}
            trafficWindowSecs={trafficWindowSecs}
            columns={COLUMNS}
            colour={colourOf(endpoint.key)}
            shape={shapeOf(endpoint.key)}
            raised={highlighted === endpoint.key}
            dimmed={highlighted !== null && highlighted !== endpoint.key}
            pinned={pinned === endpoint.key}
            onPin={() => {
              // The place it is being pinned *at*, so pinning itself never moves it.
              onPin(endpoint.key, index);
            }}
            onHover={onHover}
          />
        ))}
      </tbody>
    </table>
  );

  const heading = (
    <>
      <span className="nm-endpointgroup__title">
        <MetricHelp topic={groupTopic(group.transport)}>{t(groupKey(group.transport))}</MetricHelp>
      </span>
      <Distribution
        counts={group.counts}
        label={t('apps.distribution.group', { group: t(groupKey(group.transport)) })}
        className="nm-distribution--inline"
      />
    </>
  );

  // A group with nothing in it is not drawn at all — not even its heading and its count chip.
  // An application that only ever speaks TCP was showing an empty `UDP flows` heading, which
  // reads as "your game has no match traffic" about an application that never had any.
  //
  // The one exception is knowledge that is *missing* rather than absent: without the one-time
  // tracing setup there are no UDP endpoints on this machine whatever the application does,
  // and that is a finding with something to do about it. It keeps its sentence.
  if (group.endpoints.length === 0) {
    if (!isMatchTraffic || flowStatus === 'active') return null;
    return (
      <section className="nm-endpointgroup">
        <header className="nm-endpointgroup__header">{heading}</header>
        <p className="nm-state--pending">{t('apps.group.udpUnobservable')}</p>
      </section>
    );
  }

  if (isMatchTraffic) {
    return (
      <section className="nm-endpointgroup">
        <header className="nm-endpointgroup__header">{heading}</header>
        {rows}
      </section>
    );
  }

  return (
    <details
      className="nm-endpointgroup"
      open={open}
      onToggle={(event) => {
        setOpen(event.currentTarget.open);
      }}
    >
      <summary className="nm-endpointgroup__header">{heading}</summary>
      {rows}
    </details>
  );
};
