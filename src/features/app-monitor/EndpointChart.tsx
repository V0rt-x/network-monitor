import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import uPlot from 'uplot';

import { useFigures } from '../../shared/useFigures';
import type { ChartLine, ChartReadingEntry } from './chartSeries';
import { alignSeries, formatAxisElapsed, formatAxisMs, readingAt } from './chartSeries';

interface EndpointChartProps {
  /**
   * Seconds since monitoring began for each slot — ascending, shared by every line.
   *
   * The whole ladder arrives from the first moment, even before there is anything to draw in
   * most of it. That is what fixes the width of the axis, so a fresh application's line grows
   * rightwards into it instead of being stretched across it.
   */
  readonly elapsedSecs: readonly (number | null)[];
  readonly lines: readonly ChartLine[];
  /** The endpoint whose lines are raised, and whose siblings are dimmed. */
  readonly highlighted: string | null;
  /** Called with the endpoint under the cursor, or `null` when it leaves. */
  readonly onHover: (endpoint: string | null) => void;
  /** Called when an endpoint is chosen, which pins it and brings its row into view. */
  readonly onSelect: (endpoint: string) => void;
  /** Accessible description; every figure it draws is also in the list beside it. */
  readonly label: string;
}

const HEIGHT = 200;

/** How dim a line gets while another endpoint is raised. */
const DIMMED_ALPHA = '38';

/**
 * How much lighter a supporting connection is drawn than the match traffic.
 *
 * Emphasis, not a verdict: a TCP endpoint that is failing is a first-class finding and the
 * list beside the chart says so in words. This only stops a launcher's connection competing
 * for attention with the flow the game is played over.
 */
const SUPPORTING_ALPHA = 'b0';

/** How near the cursor has to be, in pixels, for a line to be considered hovered. */
const HOVER_PROXIMITY = 24;

/** Room reserved for an axis's name, in pixels. */
const AXIS_LABEL_SIZE = 20;

/** uPlot draws to a canvas, so an axis name needs a font rather than a class. */
const AXIS_LABEL_FONT = '12px "Segoe UI", system-ui, sans-serif';

/**
 * The chart's own share of the palette, spelled out because a canvas cannot take a class.
 *
 * Kept in step with `--nm-text-secondary` and `--nm-border` in `styles.css` by hand. Reading
 * them off the document at draw time was considered and rejected: it is a layout read on a
 * path that runs whenever the set of lines changes, for two colours that change when the
 * stylesheet does and never otherwise.
 */
const AXIS_STROKE = '#b1bac4';
const GRID_STROKE = '#30363d';

/** How wide the tooltip is, in pixels, for deciding which side of the cursor it goes on. */
const TOOLTIP_WIDTH = 260;

/**
 * Every endpoint of one application on one time axis, and a surface that can be read.
 *
 * The page's list of rows answers "how is this endpoint". The question a user actually has
 * during a match is "which of these is the odd one out", and only a shared axis answers it:
 * sixteen separate sparklines can be scanned but not compared.
 *
 * Rules it exists to keep:
 *
 * **The silent endpoint is on the chart.** A game's match server answers nothing, so it has
 * no round trip to draw — and it is the endpoint the whole product is for. It appears as the
 * route to it, dashed and named as the route, sharing the axis but never the meaning. A
 * dashed line at 40 ms is not a 40 ms ping to the server; the row beside it says so in
 * words, the chart says so by not drawing it like the others, and the tooltip says so by
 * naming its quantity *route*. The word *ping* appears on no route entry anywhere.
 *
 * **Gaps stay gaps.** uPlot does not span a `null`, so an outage is a break in the line
 * rather than a straight segment drawn through it — and a slot with nothing in it reads
 * *no reply* rather than `0` or a silently missing tooltip entry.
 *
 * **Colour identifies, it never states.** The list, ordered worst first, is the authority on
 * health. A line's colour says which endpoint it is and nothing else, and the tooltip carries
 * no word about health for the same reason: a second opinion would contradict the first the
 * moment two endpoints shared a state.
 *
 * **The chart is a reading surface, not a picture.** It had no legend, no cursor point and no
 * tooltip, and the only effect of hovering a line happened somewhere else — a row that might
 * be off screen. Now: a tooltip listing every line at the pointed moment, a crosshair saying
 * which moment that is, selection that pins the endpoint *and* brings its row into view, and
 * the same keyboard the check strip on the services page already had. Scrolling happens on
 * selection only; scrolling on hover is nauseating.
 *
 * **Nothing here drives sampling.** Rust emits at its own rate and emits nothing at all
 * while the window is hidden, and the tooltip and crosshair are drawn only in response to a
 * pointer or a key — so in the tray this costs exactly nothing.
 */
export const EndpointChart = ({
  elapsedSecs,
  lines,
  highlighted,
  onHover,
  onSelect,
  label,
}: EndpointChartProps) => {
  const { t } = useTranslation();
  const figures = useFigures();
  const keyboardId = useId();
  const host = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  // Which moment is being read, and which line inside it. `null` is nobody pointing at or
  // arrowing through the chart, which is the state it spends most of its life in.
  const [slot, setSlot] = useState<number | null>(null);
  const [focused, setFocused] = useState(0);
  // Where to put the tooltip, in pixels from the left of the chart's frame.
  const [cursorX, setCursorX] = useState(0);

  const aligned = useMemo(() => alignSeries(elapsedSecs, lines), [elapsedSecs, lines]);
  const slots = aligned[0]?.length ?? 0;

  // Read by uPlot's stroke callbacks at draw time. Kept in refs so that hovering a line
  // recolours the chart with a redraw rather than by tearing it down and rebuilding it.
  const currentLines = useRef<readonly ChartLine[]>(lines);
  currentLines.current = lines;
  const currentHighlight = useRef<string | null>(highlighted);
  currentHighlight.current = highlighted;
  const currentAligned = useRef(aligned);
  currentAligned.current = aligned;

  // The chart is rebuilt when the *set* of lines changes — an endpoint appearing or going
  // away — and only then. New numbers for the same endpoints are a `setData`, which is what
  // happens once a second.
  const shape = lines.map((line) => `${line.endpoint}${line.isPath ? ':path' : ''}`).join('|');

  useEffect(() => {
    const element = host.current;
    if (!element) return undefined;

    setFailure(null);
    const drawn = currentLines.current;
    const stroke = (index: number) => () => {
      const line = currentLines.current[index];
      if (!line) return 'transparent';
      const raised = currentHighlight.current;
      // Dimmed by alpha rather than by a grey, so a line the user is not looking at keeps
      // its identity and can still be picked out.
      if (raised !== null && raised !== line.endpoint) return `${line.colour}${DIMMED_ALPHA}`;
      return line.transport === 'tcp' ? `${line.colour}${SUPPORTING_ALPHA}` : line.colour;
    };

    let chart: uPlot | null = null;
    try {
      chart = new uPlot(
        {
          width: element.clientWidth || 640,
          height: HEIGHT,
          legend: { show: false },
          cursor: {
            // Proximity focus is what makes "the line under the pointer" a question uPlot
            // can answer at all; without it every series is always focused.
            focus: { prox: HOVER_PROXIMITY },
            points: { show: false },
            // The vertical crosshair, and no horizontal one: which *moment* the tooltip is
            // about is the thing that needs saying, and a value line across a logarithmic
            // axis invites reading a number off the axis that the tooltip already states.
            x: true,
            y: false,
          },
          scales: {
            x: { time: false },
            // Round-trip times on one chart span two orders of magnitude — a hop inside the
            // ISP at 4 ms beside a server across an ocean at 200 ms — and one spike to 400
            // flattens everything else against the floor. A logarithmic axis gives every
            // line the same vertical room to move in, which is what makes "which of these
            // is the odd one out" answerable at a glance and what makes a line possible to
            // put a pointer on. Nothing is clipped and no outlier is hidden; only the
            // spacing changes.
            y: {
              distr: 3,
              log: 10,
              // Fitted to the data rather than rounded out to whole decades: a chart whose
              // lines all sit at 150 ms must not spend half its height on the empty space
              // between 1 ms and 100 ms.
              range: (_chart, min, max) => uPlot.rangeLog(min, max, 10, false),
            },
          },
          axes: [
            {
              stroke: AXIS_STROKE,
              // No rules across the plot. They were the loudest ink on the card and carried
              // nothing: the y axis is logarithmic, so they do not even fall at round
              // distances, and what a reader actually places a value with is the crosshair,
              // which is already there. The tick marks and the labels stay.
              grid: { show: false },
              ticks: { stroke: GRID_STROKE },
              // Time since monitoring began, as minutes and seconds. Not an age: the axis is
              // anchored at the start, which is what lets the drawing grow from the left.
              values: (_chart, splits) =>
                (splits as unknown as (number | null)[]).map((value) => formatAxisElapsed(value)),
              label: t('apps.chart.axisElapsed'),
              labelSize: AXIS_LABEL_SIZE,
              labelFont: AXIS_LABEL_FONT,
            },
            {
              stroke: AXIS_STROKE,
              grid: { show: false },
              ticks: { stroke: GRID_STROKE },
              // Plain milliseconds rather than uPlot's exponent notation: the reader is
              // comparing latencies, not reading a science plot.
              values: (_chart, splits) =>
                (splits as unknown as (number | null)[]).map((value) => formatAxisMs(value)),
              // The unit, once, as the axis's own name rather than on every tick. Without it
              // the vertical scale was bare numbers — the same defect the figures had.
              label: t('apps.chart.axisMs'),
              labelSize: AXIS_LABEL_SIZE,
              labelFont: AXIS_LABEL_FONT,
            },
          ],
          series: [
            {},
            ...drawn.map((line, index) => ({
              label: line.label,
              stroke: stroke(index),
              // Thinner for a route, thinner again for a supporting connection: the match
              // traffic is what the page is emphasising and the chart agrees with it.
              width: (line.isPath ? 1.5 : 2) * (line.transport === 'tcp' ? 0.75 : 1),
              // The route is dashed, always. It is the one line on this chart that is not a
              // round trip to the thing it is named after.
              ...(line.isPath ? { dash: [4, 4] } : {}),
              points: { show: false },
            })),
          ],
          hooks: {
            // Which moment the pointer is over. This is what turns the chart from a picture
            // into something that can be read: the tooltip is built from the slot, not from
            // whichever line happened to be nearest.
            setCursor: [
              (self: uPlot) => {
                const index = self.cursor.idx;
                setSlot(index ?? null);
                setCursorX((self.cursor.left ?? 0) + self.over.offsetLeft);
              },
            ],
            setSeries: [
              (_chart: uPlot, seriesIdx: number | null) => {
                if (seriesIdx === null) return;
                // uPlot's series indices are one ahead of ours: index zero is the x axis.
                setFocused(seriesIdx - 1);
              },
            ],
          },
        },
        // Built with the data it will draw, not with empty arrays. A logarithmic scale has
        // no range to compute from nothing, and uPlot decides its scales at construction.
        currentAligned.current as unknown as uPlot.AlignedData,
        element,
      );
    } catch (error) {
      // A headless renderer with no canvas, a WebView that lost its surface, or data no
      // scale can place. Said out loud rather than swallowed: a chart that silently is not
      // there looks exactly like a chart that is broken, and the note underneath goes on
      // describing one.
      setFailure(error instanceof Error ? error.message : String(error));
      chart = null;
    }
    plot.current = chart;

    return () => {
      chart?.destroy();
      plot.current = null;
    };
    // `t` is in here because the axis names are translated and uPlot bakes them in at
    // construction: a language change has to rebuild the chart, not leave English on it.
  }, [shape, t]);

  useEffect(() => {
    const chart = plot.current;
    if (!chart) return;
    if ((aligned[0]?.length ?? 0) === 0) return;
    chart.setData(aligned as unknown as uPlot.AlignedData);
  }, [aligned]);

  // A redraw rather than a rebuild: the data has not changed, only which line is raised.
  useEffect(() => {
    plot.current?.redraw();
  }, [highlighted]);

  const reading = slot === null ? null : readingAt(aligned, lines, slot);
  const entries = reading?.entries ?? [];

  // Whether the tooltip hangs above the cursor rather than below it. Sixteen endpoints make a
  // tall list, and a chart near the bottom of the window put it past the bottom edge — the
  // same defect the explanation panels had at the right edge, and it is fixed the same way:
  // measured after layout and before paint, always from the unflipped position so it cannot
  // oscillate, and left alone where a renderer reports no layout at all.
  const tip = useRef<HTMLDivElement>(null);
  const [tipAbove, setTipAbove] = useState(false);
  useLayoutEffect(() => {
    if (reading === null || entries.length === 0) {
      setTipAbove(false);
      return;
    }
    const box = tip.current?.getBoundingClientRect();
    if (box === undefined || box.height === 0) return;
    // Only when there is room above. Flipping a list taller than the window would trade one
    // clipped end for the other.
    setTipAbove(box.bottom > window.innerHeight && box.height < box.top);
  }, [reading, entries.length]);
  // Which entry the reader is on. The focused *line* is an index into `lines`; the tooltip
  // is sorted worst first, so it has to be found rather than indexed.
  const focusedEndpoint = lines[focused]?.endpoint ?? null;

  /** How one line's value at this moment is written, in the tooltip and in the readout. */
  const entryText = (entry: ChartReadingEntry): string => {
    if (entry.valueMs === null) return t('apps.chart.entryNone', { endpoint: entry.address });
    const value = figures.ms(entry.valueMs);
    // The never-merge rule, applied to the tooltip. A route is a round trip to a router
    // short of the endpoint, and calling it a ping here would undo everything the dashed
    // line and the row's own labels do.
    return entry.isPath
      ? t('apps.chart.entryRoute', { endpoint: entry.address, value })
      : t('apps.chart.entryPing', { endpoint: entry.address, value });
  };

  /** Moves the read moment, opening at the newest slot when nothing was being read. */
  const stepSlot = (by: number) => {
    setSlot((current) => {
      if (slots === 0) return null;
      const next = (current ?? slots - 1) + by;
      return Math.min(Math.max(next, 0), slots - 1);
    });
  };

  /** Moves between lines, wrapping, so a keyboard reader can reach all of them. */
  const stepLine = (by: number) => {
    if (lines.length === 0) return;
    const next = (focused + by + lines.length) % lines.length;
    setFocused(next);
    onHover(lines[next]?.endpoint ?? null);
  };

  // Keeps the crosshair with the keyboard. uPlot draws its cursor where it was last told
  // the pointer was, so arrowing through time without this would move the tooltip and leave
  // the line behind.
  useEffect(() => {
    const chart = plot.current;
    if (!chart || slot === null) return;
    const x = aligned[0]?.[slot];
    if (x === null || x === undefined) return;
    const left = chart.valToPos(x, 'x');
    if (!Number.isFinite(left)) return;
    setCursorX(left + chart.over.offsetLeft);
  }, [slot, aligned]);

  return (
    <>
      <div
        className="nm-endpointchart__frame"
        // Let go of the moment only when focus leaves the chart *and* its tooltip. Clearing
        // it on the chart's own blur closed the tooltip the instant a reader tabbed or
        // clicked into it — which is to say, exactly when they were choosing an entry.
        onBlur={(event) => {
          if (event.currentTarget.contains(event.relatedTarget)) return;
          setSlot(null);
        }}
      >
        <div
          className="nm-endpointchart"
          ref={host}
          // One tab stop for the whole chart, with the arrows inside it — the same shape the
          // check strip on the services page has had all along, and which the far more
          // important surface here did not.
          tabIndex={0}
          role="img"
          aria-label={label}
          // How to operate it, said to whoever lands on it and to nobody else. A standing
          // instruction printed under every chart is the prose the page exists not to carry.
          aria-describedby={keyboardId}
          onMouseLeave={() => {
            onHover(null);
            setSlot(null);
          }}
          onClick={() => {
            // uPlot has already told us which line the cursor is nearest; a click pins it
            // and brings its row into view.
            if (focusedEndpoint !== null) onSelect(focusedEndpoint);
          }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowLeft') stepSlot(-1);
            else if (event.key === 'ArrowRight') stepSlot(1);
            else if (event.key === 'ArrowUp') stepLine(-1);
            else if (event.key === 'ArrowDown') stepLine(1);
            else if (event.key === 'Home') setSlot(0);
            else if (event.key === 'End') setSlot(slots === 0 ? null : slots - 1);
            else if (event.key === 'Enter') {
              if (focusedEndpoint !== null) onSelect(focusedEndpoint);
            } else if (event.key === 'Escape') {
              setSlot(null);
              onHover(null);
            } else return;
            // Only for the keys the chart handled: the page must still scroll on the rest.
            event.preventDefault();
          }}
        />

        {/* The legend the chart never had, the readout of the moment, and the way into the
            row — one thing rather than three. It lists every line, because "which of these
            is the odd one out" is a question about all of them at that second. */}
        {reading !== null && entries.length > 0 && (
          <div
            className={tipAbove ? 'nm-charttip nm-charttip--above' : 'nm-charttip'}
            role="presentation"
            ref={tip}
            style={
              // Flipped at the right-hand edge, exactly as the explanation panels are: a
              // tooltip half outside the window is the one thing on screen that cannot be
              // read.
              cursorX + TOOLTIP_WIDTH > (host.current?.clientWidth ?? 0)
                ? { right: `${String(Math.max(0, (host.current?.clientWidth ?? 0) - cursorX))}px` }
                : { left: `${String(cursorX)}px` }
            }
          >
            <p className="nm-charttip__at">
              {t('apps.chart.at', { time: formatAxisElapsed(reading.elapsedSecs) })}
            </p>
            <ul className="nm-charttip__lines">
              {entries.map((entry) => (
                <li
                  key={`${entry.endpoint}${entry.isPath ? ':path' : ''}`}
                  className={
                    entry.endpoint === focusedEndpoint
                      ? 'nm-charttip__line nm-charttip__line--focused'
                      : 'nm-charttip__line'
                  }
                >
                  <button
                    type="button"
                    onClick={(event) => {
                      // The chart's own click handler would fire too and pin whatever the
                      // pointer happened to be nearest instead of what was chosen.
                      event.stopPropagation();
                      onSelect(entry.endpoint);
                    }}
                  >
                    <span
                      className="nm-charttip__swatch"
                      style={{ backgroundColor: entry.colour }}
                      aria-hidden="true"
                    />
                    {entryText(entry)}
                  </button>
                </li>
              ))}
            </ul>
            <p className="nm-charttip__hint">{t('apps.chart.pinHint')}</p>
          </div>
        )}
        <span id={keyboardId} className="nm-visually-hidden">
          {t('apps.chart.keyboard')}
        </span>
      </div>

      {/* Polite rather than assertive: a reading the user asked for by pointing or by
          arrowing, not an announcement that should interrupt. The focused line is spoken
          first and the rest follow, so the region carries what the tooltip shows without
          making a keyboard reader wait through fifteen lines for the one they moved to. */}
      <p className="nm-endpointchart__readout" role="status">
        {reading === null
          ? ''
          : [
              t('apps.chart.at', { time: formatAxisElapsed(reading.elapsedSecs) }),
              ...[...entries]
                .sort((left, right) =>
                  left.endpoint === focusedEndpoint
                    ? -1
                    : right.endpoint === focusedEndpoint
                      ? 1
                      : 0,
                )
                .map(entryText),
            ].join(' · ')}
      </p>

      {failure !== null && (
        <p className="nm-state--degraded" role="alert">
          {t('apps.chart.failed', { reason: failure })}
        </p>
      )}
    </>
  );
};
