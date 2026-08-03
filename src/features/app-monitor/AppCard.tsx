import { useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { AppView, FlowStatusView } from '../../shared/ipc';
import { VerdictBanner } from '../../shared/VerdictBanner';
import type { ChartLine } from './chartSeries';
import { rowIdOf } from './chartSeries';
import { Distribution } from './Distribution';
import { EndpointChart } from './EndpointChart';
import { EndpointColours } from './endpointColours';
import { EndpointGroup } from './EndpointGroup';
import { MetricHelp } from '../help/MetricHelp';
import { PoolPanel } from './PoolPanel';
import { PrimaryFlow } from './PrimaryFlow';
import { WhyNotYourPing } from './WhyNotYourPing';

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
 * The processes it consists of are **counted, and never named**. *Changed on the user's
 * instruction on 2026-08-04, and it amends Phase 4's "a grouping the user cannot inspect is
 * one they cannot correct".* What survives of that requirement is the count — it says how
 * large a group the rule caught, which is what would look wrong if the grouping were wrong.
 * What goes is the per-process identity at every level, expander included: a player picks
 * *Discord*, and `Discord.exe` beside it and `PID 25572` beside that are the product's
 * implementation showing through. Rust no longer sends them, so no level of this can show
 * them.
 *
 * An empty group stays at level one and stays a sentence, because it is a finding rather than
 * a detail: the application was chosen and nothing is running under it, which is exactly what
 * arming the monitor before a match looks like, and a bare "0 processes" would read as a bug.
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

  // Pinned by a click or by activating a row; cleared by picking it again. The index it held
  // when it was pinned travels with it: a pin that moved the row would be the same jump the
  // pin exists to prevent.
  const [pinned, setPinned] = useState<{ key: string; at: number } | null>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const highlighted = hovered ?? pinned?.key ?? null;

  // View state, not a measurement — but it has to survive the re-render new data causes, or
  // every line would change colour once a second.
  const colours = useRef(new EndpointColours());

  // The chart draws every endpoint of the application, groups or no groups: "which of these
  // is the odd one out" is a question about all of them at once. The grouping reaches it as
  // emphasis — the match traffic at full weight — and never as an omission.
  const endpoints = useMemo(() => app.groups.flatMap((group) => group.endpoints), [app.groups]);

  // Whether anything here answers nothing our probes can send. That is the normal state of a
  // game's match server rather than a fault — nothing listens on a game port but the game —
  // and it is the case the whole product exists for, so the card says once why what it shows
  // instead is not the number the game shows.
  const primary = endpoints.find((endpoint) => endpoint.key === app.primaryEndpoint);

  const answersNothing = endpoints.some(
    (endpoint) => !endpoint.probesMeasureIt && (endpoint.path !== null || endpoint.flow !== null),
  );

  /**
   * Pins an endpoint where it currently sits, or unpins it.
   *
   * The place is recorded with the key because that is the whole point: a pin that moved the
   * row to the top would be the same jump the pin exists to prevent. Picking the same
   * endpoint again releases it.
   */
  const pin = (key: string, at?: number) => {
    const index =
      at ??
      app.groups.reduce((found, group) => {
        const inside = group.endpoints.findIndex((endpoint) => endpoint.key === key);
        return inside === -1 ? found : inside;
      }, 0);
    setPinned((current) => (current?.key === key ? null : { key, at: index }));
  };

  /**
   * Pins an endpoint chosen on the chart, and brings its row into view.
   *
   * The complaint this answers: hovering a line highlighted a row that might be off screen,
   * so the only effect of touching the chart happened somewhere the reader was not looking.
   * On selection only — scrolling on hover is nauseating — and `nearest`, so a row already
   * visible does not move at all.
   */
  const chooseFromChart = (key: string) => {
    pin(key);
    const row = document.getElementById(rowIdOf(app.id, key));
    if (typeof row?.scrollIntoView === 'function') row.scrollIntoView({ block: 'nearest' });
  };

  const lines = useMemo(() => {
    colours.current.reconcile(endpoints.map((endpoint) => endpoint.key));
    const drawn: ChartLine[] = [];
    for (const endpoint of endpoints) {
      const colour = colours.current.of(endpoint.key);
      if (endpoint.chartRttMs.some((value) => value !== null)) {
        drawn.push({
          endpoint: endpoint.key,
          address: endpoint.address,
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
          address: endpoint.address,
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
          {/* A count, and nothing else. The expander used to enumerate `Discord.exe · PID
              25572` seventeen times over — three restatements of a fact the reader did not
              ask for, and the product's implementation showing through. The count is the
              part that is worth a glance, because it is what would look wrong if the
              grouping were; the identities do not cross the IPC boundary at all now.

              An empty group keeps a sentence rather than reading "0 processes", because it
              is a finding: the monitor is armed and the game has not started. */}
          {app.pids.length === 0 ? (
            <p className="nm-appcard__processes nm-state--pending">{t('apps.processes.none')}</p>
          ) : (
            <p className="nm-appcard__processes">
              {t('apps.processes.count', { count: app.pids.length })}
            </p>
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

      {/* The first window after an application is chosen says nothing about it — but the
          network underneath it has been measured all along, so the banner still reports
          that. Rust decides both, by simply not offering this application as evidence yet. */}
      {app.warmupSecsRemaining !== null && (
        <p className="nm-appcard__warmup nm-state--pending">
          <MetricHelp topic="warmup">
            {t('apps.warmup.application', { seconds: Math.ceil(app.warmupSecsRemaining) })}
          </MetricHelp>
        </p>
      )}

      <VerdictBanner diagnosis={app.diagnosis} subject={app.name} />
      <PoolPanel pool={app.pool} />

      <Distribution counts={app.counts} label={t('apps.distribution.application')} />

      {/* The card leads with the endpoint the user came for, where the measurement supports
          naming one. Where two flows are close enough that it would not, Rust says so and the
          card leads with the table alone rather than picking a winner by a tiebreak. */}
      {primary === undefined ? (
        <p className="nm-primary__none">{t('apps.primary.none')}</p>
      ) : (
        <PrimaryFlow endpoint={primary} trafficWindowSecs={trafficWindowSecs} />
      )}

      {/* The chart and its caption are one block with a tighter gap of its own, so the card's
          own rhythm does not push a caption away from the picture it captions. */}
      {lines.length > 0 && (
        <div className="nm-appcard__chart">
          <EndpointChart
            elapsedSecs={app.chartElapsedSecs}
            lines={lines}
            highlighted={highlighted}
            onHover={setHovered}
            onSelect={chooseFromChart}
            label={t('apps.chart.label', { name: app.name })}
          />
          {/* A caption, not a paragraph. Six sentences of drawing decisions — the log
              scale, the slot width, the slot maxima, what a break means — used to sit here
              competing with the chart they described; they are the help's job, and what a
              reader needs beside the picture is what the axes are. */}
          <p className="nm-appcard__chartnote">
            <MetricHelp topic="chart">
              {t('apps.chart.caption', { seconds: chartStepSecs })}
            </MetricHelp>
          </p>
          {/* Once per card, under the chart it is about — not once per silent endpoint.
              It is the same three-point disclosure every time, and a game has several
              silent endpoints, so it appeared six times on one card saying the same thing.
              A reader who has had the explanation does not need it again six rows later. */}
          {answersNothing && <WhyNotYourPing />}
        </div>
      )}

      {endpoints.length === 0 && <p className="nm-state--pending">{t('apps.noEndpoints')}</p>}

      {app.groups.map((group) => (
        <EndpointGroup
          key={group.transport}
          appId={app.id}
          group={group}
          flowStatus={flowStatus}
          trafficWindowSecs={trafficWindowSecs}
          colourOf={(endpoint) => colours.current.of(endpoint)}
          shapeOf={(endpoint) => colours.current.shapeOf(endpoint)}
          highlighted={highlighted}
          pinned={pinned?.key ?? null}
          pinnedAt={pinned?.at ?? null}
          onPin={pin}
          onHover={setHovered}
        />
      ))}
    </section>
  );
};
