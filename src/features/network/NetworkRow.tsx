import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { NetworkRowView } from '../../shared/ipc';
import { MetricRow } from '../../shared/MetricRow';
import { StateToken } from '../../shared/StateToken';
import { useFigures } from '../../shared/useFigures';
import { Distribution } from '../app-monitor/Distribution';
import { probeKindKey } from '../dashboard/labels';
import { MetricHelp } from '../help/MetricHelp';
import { CheckTimeline } from '../status-page/CheckTimeline';

interface NetworkRowProps {
  readonly row: NetworkRowView;
  /** How often this row's section is checked, for the staleness rule. */
  readonly cadenceSecs: number;
}

/**
 * One row of the Network page: a name, a state, its recent checks, and its round trip.
 *
 * **One component for the whole page.** A baseline target and a gaming platform were drawn
 * by two components in two visual languages — `GroupCard`+`TargetRow` beside
 * `ServiceCard`+`EndpointRow` — over two IPC shapes, with two distribution renderings that
 * shared their CSS. They are one object: a named thing with one or more addresses.
 *
 * **The strip is what survived, and the sparkline is what went.** `Sparkline` drew round-trip
 * time over time and *stroked it in a colour that stated health*, which is the one rule about
 * colour this product does not break anywhere else. `CheckTimeline` draws one cell per check
 * and names six distinguishable outcomes in words. For a page whose question is "which of
 * these is red", the strip is strictly the better instrument, and it carries more rather than
 * less. Continuous round-trip reading belongs to the applications chart, where it already is.
 *
 * **Folded to one line, and it says which line.** Twenty-three rows each carrying a strip and
 * three figures per endpoint turns "which of these is red" into a scrolling task. Closed: the
 * name, one word of state, the strip and `Ping (RTT)`. The endpoints, the badges and the
 * remaining figures are level two.
 *
 * A row worse than clean starts open — as the *initial* state only. Re-applying that on every
 * render is what made the applications page open and collapse under the reader, because the
 * value flips constantly on a weak link.
 */
export const NetworkRow = ({ row, cadenceSecs }: NetworkRowProps) => {
  const { t } = useTranslation();
  const figures = useFigures();
  const [open, setOpen] = useState(row.health !== 'ok');

  const lastChecked =
    row.lastCheckedSecs === null
      ? t('status.neverChecked')
      : t('status.lastChecked', { seconds: Math.round(row.lastCheckedSecs) });
  // A check older than two whole intervals is not merely aging — something has stopped, and a
  // page whose data quietly stopped arriving looks exactly like one reporting calm.
  const stale = row.lastCheckedSecs !== null && row.lastCheckedSecs > cadenceSecs * 2;

  // The first endpoint's strip stands for the row when it is closed. A row with two of them
  // shows both once it is opened; picking one to summarise the other would be a claim.
  const summary = row.endpoints[0];

  return (
    <details
      className="nm-row"
      open={open}
      onToggle={(event) => {
        setOpen(event.currentTarget.open);
      }}
    >
      <summary className="nm-row__head">
        <span className="nm-row__label">{row.label}</span>
        <StateToken health={row.health} />
        <span className="nm-row__strip">
          {summary !== undefined && (
            <CheckTimeline
              checks={summary.checks}
              label={t('status.timelineLabel', {
                service: row.label,
                address: summary.writtenAddress,
              })}
            />
          )}
        </span>
        {/* One of exactly two names this page gives a round trip. The other is
            `Ping, median` on a section heading; `Ping, last check` and `Ping, mean` are a
            *which window* qualifier and live a level down. There were five. */}
        <span className="nm-row__rtt">{figures.ms(row.rttMs)}</span>
      </summary>

      <div className="nm-row__detail">
        <p className={stale ? 'nm-service__stale' : 'nm-service__checked'}>{lastChecked}</p>

        {/* Shown whenever the row has more than one endpoint: a storefront answering while
            the gateway does not is the finding, and one amber dot would hide which half. */}
        {row.endpoints.length > 1 && (
          <Distribution counts={row.counts} label={t('network.distribution', { row: row.label })} />
        )}

        <ul className="nm-row__endpoints">
          {row.endpoints.map((endpoint) => (
            <li key={endpoint.key} className="nm-row__endpoint">
              <div className="nm-row__endpointhead">
                <span className="nm-service__address" title={endpoint.resolvedAddress ?? undefined}>
                  {endpoint.writtenAddress}
                </span>
                <div className="nm-endpoint__badges">
                  {/* The tunnel travels with the state as a qualifier rather than as a pill
                      of its own: it is not a fault and there is nothing to do about it — it
                      changes what the figures beside it *mean*, which is the test. With a
                      VPN running it is on nearly every row on the page. */}
                  {row.endpoints.length > 1 && (
                    <StateToken
                      health={endpoint.health}
                      qualifiers={
                        endpoint.tunnelled
                          ? [{ kind: 'tunnelled', name: t('dashboard.badge.tunnelled') }]
                          : []
                      }
                    />
                  )}
                  {endpoint.probeKind !== null && (
                    <span className="nm-badge">{t(probeKindKey(endpoint.probeKind))}</span>
                  )}
                  {/* A row with one endpoint carries no state token, so the tunnel has to be
                      said here or not at all. */}
                  {row.endpoints.length === 1 && endpoint.tunnelled && (
                    <span className="nm-badge">
                      <MetricHelp topic="tunnel">{t('dashboard.badge.tunnelled')}</MetricHelp>
                    </span>
                  )}
                  {endpoint.filteringConfirmed && (
                    <span className="nm-badge">{t('dashboard.badge.filteringConfirmed')}</span>
                  )}
                  {endpoint.resolvedAddress === null && (
                    <span className="nm-badge nm-badge--warn">
                      {t('dashboard.badge.unresolved')}
                    </span>
                  )}
                  {endpoint.resolvedAddress !== null && !endpoint.measurable && (
                    <span className="nm-badge nm-badge--warn">
                      {t('dashboard.badge.notMeasurable')}
                    </span>
                  )}
                </div>
              </div>

              {row.endpoints.length > 1 && (
                <CheckTimeline
                  checks={endpoint.checks}
                  label={t('status.timelineLabel', {
                    service: row.label,
                    address: endpoint.writtenAddress,
                  })}
                />
              )}

              {/* *Which window* a figure covers is the qualifier that moved down a level, and
                  both names stay because the distinction is real and worth explaining where
                  it is made. */}
              <MetricRow
                metrics={[
                  {
                    key: 'latest',
                    label: <MetricHelp topic="latestCheck">{t('status.metric.latest')}</MetricHelp>,
                    value: figures.ms(endpoint.rttMs),
                  },
                  {
                    key: 'mean',
                    label: <MetricHelp topic="meanRtt">{t('status.metric.mean')}</MetricHelp>,
                    value: figures.ms(endpoint.meanRttMs),
                  },
                  {
                    key: 'loss',
                    label: <MetricHelp topic="loss">{t('status.metric.loss')}</MetricHelp>,
                    value: figures.pct(endpoint.lossPct),
                  },
                ]}
              />
            </li>
          ))}
        </ul>
      </div>
    </details>
  );
};
