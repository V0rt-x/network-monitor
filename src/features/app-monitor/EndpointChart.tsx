import { useEffect, useRef } from 'react';
import uPlot from 'uplot';

import type { ChartLine } from './chartSeries';
import { alignSeries } from './chartSeries';

interface EndpointChartProps {
  /** Seconds before now for each slot — negative, ascending, shared by every line. */
  readonly ageSecs: readonly (number | null)[];
  readonly lines: readonly ChartLine[];
  /** The endpoint whose lines are raised, and whose siblings are dimmed. */
  readonly highlighted: string | null;
  /** Called with the endpoint under the cursor, or `null` when it leaves. */
  readonly onHover: (endpoint: string | null) => void;
  /** Called when a line is clicked, which pins the selection. */
  readonly onSelect: (endpoint: string) => void;
  /** Accessible description; every figure it draws is also in the list beside it. */
  readonly label: string;
}

const HEIGHT = 200;

/** How dim a line gets while another endpoint is raised. */
const DIMMED_ALPHA = '38';

/** How near the cursor has to be, in pixels, for a line to be considered hovered. */
const HOVER_PROXIMITY = 24;

/**
 * Every endpoint of one application on one time axis.
 *
 * The page's list of rows answers "how is this endpoint". The question a user actually has
 * during a match is "which of these is the odd one out", and only a shared axis answers it:
 * sixteen separate sparklines can be scanned but not compared.
 *
 * Four rules it exists to keep:
 *
 * **The silent endpoint is on the chart.** A game's match server answers nothing, so it has
 * no round trip to draw — and it is the endpoint the whole product is for. It appears as the
 * route to it, dashed and named as the route, sharing the axis but never the meaning. A
 * dashed line at 40 ms is not a 40 ms ping to the server; the row beside it says so in
 * words, and the chart says so by not drawing it like the others.
 *
 * **Gaps stay gaps.** uPlot does not span a `null`, so an outage is a break in the line
 * rather than a straight segment drawn through it.
 *
 * **Colour identifies, it never states.** The list, ordered worst first, is the authority on
 * health. A line's colour says which endpoint it is and nothing else.
 *
 * **Nothing here drives sampling.** Rust emits at its own rate and emits nothing at all
 * while the window is hidden, so a chart of sixteen series costs no canvas work in the tray.
 */
export const EndpointChart = ({
  ageSecs,
  lines,
  highlighted,
  onHover,
  onSelect,
  label,
}: EndpointChartProps) => {
  const host = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);

  // Read by uPlot's stroke callbacks at draw time. Kept in refs so that hovering a line
  // recolours the chart with a redraw rather than by tearing it down and rebuilding it.
  const currentLines = useRef<readonly ChartLine[]>(lines);
  currentLines.current = lines;
  const currentHighlight = useRef<string | null>(highlighted);
  currentHighlight.current = highlighted;
  const notifyHover = useRef(onHover);
  notifyHover.current = onHover;
  const notifySelect = useRef(onSelect);
  notifySelect.current = onSelect;

  // The chart is rebuilt when the *set* of lines changes — an endpoint appearing or going
  // away — and only then. New numbers for the same endpoints are a `setData`, which is what
  // happens once a second.
  const shape = lines.map((line) => `${line.endpoint}${line.isPath ? ':path' : ''}`).join('|');

  useEffect(() => {
    const element = host.current;
    if (!element) return undefined;

    const drawn = currentLines.current;
    const stroke = (index: number) => () => {
      const line = currentLines.current[index];
      if (!line) return 'transparent';
      const raised = currentHighlight.current;
      // Dimmed by alpha rather than by a grey, so a line the user is not looking at keeps
      // its identity and can still be picked out.
      return raised === null || raised === line.endpoint
        ? line.colour
        : `${line.colour}${DIMMED_ALPHA}`;
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
          },
          scales: { x: { time: false } },
          axes: [
            { stroke: '#94a3b8', grid: { stroke: '#1e3a5f' }, ticks: { stroke: '#1e3a5f' } },
            { stroke: '#94a3b8', grid: { stroke: '#1e3a5f' }, ticks: { stroke: '#1e3a5f' } },
          ],
          series: [
            {},
            ...drawn.map((line, index) => ({
              label: line.label,
              stroke: stroke(index),
              width: line.isPath ? 1.5 : 2,
              // The route is dashed, always. It is the one line on this chart that is not a
              // round trip to the thing it is named after.
              ...(line.isPath ? { dash: [4, 4] } : {}),
              points: { show: false },
            })),
          ],
          hooks: {
            setSeries: [
              (_chart: uPlot, seriesIdx: number | null) => {
                const line = seriesIdx === null ? undefined : currentLines.current[seriesIdx - 1];
                notifyHover.current(line?.endpoint ?? null);
              },
            ],
          },
        },
        [[], ...drawn.map(() => [])] as unknown as uPlot.AlignedData,
        element,
      );
    } catch {
      // No canvas context — a headless renderer, or a WebView that lost its surface. Every
      // figure the chart would draw is in the list beside it, so it simply does not appear.
      chart = null;
    }
    plot.current = chart;

    return () => {
      chart?.destroy();
      plot.current = null;
    };
  }, [shape]);

  useEffect(() => {
    const chart = plot.current;
    if (!chart) return;
    const data = alignSeries(ageSecs, lines);
    if ((data[0]?.length ?? 0) === 0) return;
    chart.setData(data as unknown as uPlot.AlignedData);
  }, [ageSecs, lines]);

  // A redraw rather than a rebuild: the data has not changed, only which line is raised.
  useEffect(() => {
    plot.current?.redraw();
  }, [highlighted]);

  return (
    <div
      className="nm-endpointchart"
      ref={host}
      role="img"
      aria-label={label}
      onMouseLeave={() => {
        onHover(null);
      }}
      onClick={() => {
        // uPlot has already told us which line the cursor is nearest; a click pins it.
        if (highlighted !== null) onSelect(highlighted);
      }}
    />
  );
};
