import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { EndpointView } from '../../shared/ipc';
import { StateToken } from '../../shared/StateToken';
import { useFigures } from '../../shared/useFigures';
import { MetricHelp } from '../help/MetricHelp';
import { EndpointBadges } from './EndpointBadges';
import { useQualifiers } from './qualifiers';
import { EndpointDetails } from './EndpointDetails';
import { FlowPanel } from './FlowPanel';
import { networkName } from './networkName';
import { PathPanel } from './PathPanel';

interface PrimaryFlowProps {
  readonly endpoint: EndpointView;
  /** Span the byte count covers, for the expander's traffic figure. */
  readonly trafficWindowSecs: number;
}

/**
 * The endpoint carrying materially more of the application's traffic than any other.
 *
 * **A measured fact, not a role.** The card has to lead with the endpoint the user came for,
 * and `view.rs` rightly refuses to label an endpoint by role, on the grounds that everything
 * except the transport and the volume of traffic would be a guess. The volume is precisely
 * what is measured — so "the busiest flow" is a fact, while "the one the game is played over"
 * remains a guess nobody makes. Rust decides it, as every judgement in this product does, and
 * declines to decide where two flows are close enough that naming one would be a claim the
 * measurement does not support. The card then leads with the table alone.
 *
 * This is the one endpoint that keeps everything at level one: its route and its traffic in
 * full, where every other endpoint has them one level down. It is a card in a page of table
 * rows on purpose — it is the answer to the question the page was opened with.
 */
export const PrimaryFlow = ({ endpoint, trafficWindowSecs }: PrimaryFlowProps) => {
  const { t } = useTranslation();
  const figures = useFigures();
  const qualifiers = useQualifiers(endpoint);
  const [open, setOpen] = useState(false);

  return (
    <section className="nm-primary">
      <header className="nm-primary__header">
        <h4 className="nm-primary__heading">
          <MetricHelp topic="traffic">{t('apps.primary.heading')}</MetricHelp>
        </h4>
        <span className="nm-primary__address">{endpoint.address}</span>
        <span className="nm-endpoint__transport">{endpoint.transport.toUpperCase()}</span>
        {endpoint.network !== null && (
          <span className="nm-endpoint__network">{networkName(endpoint.network, t)}</span>
        )}
        <StateToken health={endpoint.health} qualifiers={qualifiers} />
        <EndpointBadges endpoint={endpoint} />
      </header>

      {/* Its own three figures, where there are any. An endpoint nothing can measure shows
          none at all rather than three dashes: the route and the traffic below stand in for
          them, and dashes where the headline figures belong read as a broken tool. */}
      {endpoint.probesMeasureIt && (
        <dl className="nm-endpoint__metrics">
          <div>
            <dt>
              <MetricHelp topic="rtt">{t('apps.metric.rtt')}</MetricHelp>
            </dt>
            <dd>{figures.ms(endpoint.rttMs)}</dd>
          </div>
          <div>
            <dt>
              <MetricHelp topic="jitter">{t('apps.metric.jitter')}</MetricHelp>
            </dt>
            <dd>{figures.ms(endpoint.jitterMs)}</dd>
          </div>
          <div>
            <dt>
              <MetricHelp topic="loss">{t('apps.metric.loss')}</MetricHelp>
            </dt>
            <dd>{figures.pct(endpoint.lossPct)}</dd>
          </div>
        </dl>
      )}

      {/* Two panels, never one merged figure — and here they are level one, because this is
          the endpoint the whole product exists to watch. */}
      {(endpoint.path !== null || endpoint.flow !== null) && (
        <div className="nm-endpoint__columns">
          {endpoint.path !== null && <PathPanel path={endpoint.path} />}
          {endpoint.flow !== null && <FlowPanel flow={endpoint.flow} />}
        </div>
      )}

      <button
        type="button"
        className="nm-primary__disclose"
        aria-expanded={open}
        onClick={() => {
          setOpen((current) => !current);
        }}
      >
        {t('apps.details.summary')}
      </button>
      {open && (
        <EndpointDetails
          endpoint={endpoint}
          trafficWindowSecs={trafficWindowSecs}
          withPanels={false}
        />
      )}
    </section>
  );
};
