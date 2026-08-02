import { useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { AppView, FlowStatusView } from '../../shared/ipc';
import { VerdictBanner } from '../../shared/VerdictBanner';
import type { ChartLine } from './chartSeries';
import { Distribution } from './Distribution';
import { EndpointChart } from './EndpointChart';
import { EndpointColours } from './endpointColours';
import { EndpointGroup } from './EndpointGroup';
import { PoolPanel } from './PoolPanel';

interface AppCardProps {
  readonly app: AppView;
  /** Span the byte counts cover. */
  readonly trafficWindowSecs: number;
  /** How much time one slot of the chart covers, so the note can say what a point is. */
  readonly chartStepSecs: number;
  /** Whether per-process flow events are running, which decides why a UDP group is empty. */
  readonly flowStatus: FlowStatusView;
  readonly onForget: (app: number) => void;
}

/**
 * One monitored application: every endpoint on one chart, and the same endpoints in a list
 * beside it, worst first.
 *
 * There is deliberately no verdict for the application as a whole. "4 clean, 2 degraded, 1
 * unreachable" is a fact the user can act on; one colour for a game is either an outage
 * that is not happening or a failure that is being hidden. Partial failure inside one
 * application is the normal case under filtering, not an edge case — its endpoints sit in
 * different networks and a tunnel may cover some of them and not others.
 *
 * The processes it currently consists of are listed rather than counted. An application is
 * a group Rust formed — the picked process, its namesakes, its descendants — and a grouping
 * the user cannot inspect is one they cannot correct. An empty list is a real state: the
 * application was chosen and nothing is running under it, which is exactly what arming the
 * monitor before a match looks like.
 *
 * **The chart is additive and the list is the authority.** Hovering a line raises its
 * endpoint's row and dims the rest rather than hiding them; a click pins that choice, and so
 * does focusing or activating a row, which is how the same thing is reached without a mouse.
 * Nothing about health is expressed by the chart alone.
 */
export const AppCard = ({
  app,
  trafficWindowSecs,
  chartStepSecs,
  flowStatus,
  onForget,
}: AppCardProps) => {
  const { t } = useTranslation();

  // Pinned by a click or by activating a row; cleared by picking it again.
  const [pinned, setPinned] = useState<string | null>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const highlighted = hovered ?? pinned;

  // View state, not a measurement — but it has to survive the re-render new data causes, or
  // every line would change colour once a second.
  const colours = useRef(new EndpointColours());

  // The chart draws every endpoint of the application, groups or no groups: "which of these
  // is the odd one out" is a question about all of them at once. The grouping reaches it as
  // emphasis — the match traffic at full weight — and never as an omission.
  const endpoints = useMemo(() => app.groups.flatMap((group) => group.endpoints), [app.groups]);

  const lines = useMemo(() => {
    colours.current.reconcile(endpoints.map((endpoint) => endpoint.key));
    const drawn: ChartLine[] = [];
    for (const endpoint of endpoints) {
      const colour = colours.current.of(endpoint.key);
      if (endpoint.chartRttMs.some((value) => value !== null)) {
        drawn.push({
          endpoint: endpoint.key,
          transport: endpoint.transport,
          label: endpoint.address,
          values: endpoint.chartRttMs,
          colour,
          isPath: false,
        });
      }
      // The silent match server's only figure. Drawn dashed and named as the route, because
      // it belongs to a router short of the endpoint and calling it the endpoint's ping is
      // the one lie this product exists not to tell.
      if (endpoint.chartPathMs.some((value) => value !== null)) {
        drawn.push({
          endpoint: endpoint.key,
          transport: endpoint.transport,
          label: t('apps.chart.pathSeries', { endpoint: endpoint.address }),
          values: endpoint.chartPathMs,
          colour,
          isPath: true,
        });
      }
    }
    return drawn;
  }, [endpoints, t]);

  return (
    <section className="nm-appcard">
      <header className="nm-appcard__header">
        <div>
          <h3 className="nm-appcard__title">{app.name}</h3>
          {app.processes.length === 0 ? (
            <p className="nm-appcard__processes nm-state--pending">{t('apps.processes.none')}</p>
          ) : (
            <ul className="nm-appcard__processes" aria-label={t('apps.processes.label')}>
              {app.processes.map((process) => (
                <li key={process.pid}>
                  {t('apps.processes.entry', { name: process.name, pid: process.pid })}
                </li>
              ))}
            </ul>
          )}
        </div>
        <button
          type="button"
          className="nm-button nm-button--quiet"
          onClick={() => {
            onForget(app.id);
          }}
        >
          {t('apps.stop')}
        </button>
      </header>

      <VerdictBanner diagnosis={app.diagnosis} subject={app.name} />
      <PoolPanel pool={app.pool} />

      <Distribution
        counts={app.counts}
        label={t('apps.distribution.application')}
        className="nm-appcard__distribution"
      />

      {lines.length > 0 && (
        <>
          <EndpointChart
            ageSecs={app.chartAgeSecs}
            lines={lines}
            highlighted={highlighted}
            onHover={setHovered}
            onSelect={setPinned}
            label={t('apps.chart.label', { name: app.name })}
          />
          <p className="nm-appcard__chartnote">
            {t('apps.chart.note', { seconds: chartStepSecs })}
          </p>
        </>
      )}

      {endpoints.length === 0 && <p className="nm-state--pending">{t('apps.noEndpoints')}</p>}

      {app.groups.map((group) => (
        <EndpointGroup
          key={group.transport}
          group={group}
          flowStatus={flowStatus}
          trafficWindowSecs={trafficWindowSecs}
          colourOf={(endpoint) => colours.current.of(endpoint)}
          highlighted={highlighted}
          pinned={pinned}
          onPin={(endpoint) => {
            setPinned((current) => (current === endpoint ? null : endpoint));
          }}
          onHover={setHovered}
        />
      ))}
    </section>
  );
};
