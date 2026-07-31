import { useEffect, useRef } from 'react';
import uPlot from 'uplot';

import type { HealthView } from '../../shared/ipc';

const WIDTH = 168;
const HEIGHT = 34;

/**
 * Stroke colour per health state.
 *
 * uPlot draws to a canvas and cannot take a CSS class, so these mirror the custom
 * properties in `styles.css` by name. They are the only place in the UI where a colour is
 * repeated, and the trade is deliberate: the alternative is reading computed styles on
 * every frame.
 */
const STROKE: Record<HealthView, string> = {
  ok: '#3fb950',
  degraded: '#d29922',
  unreachable: '#f85149',
  blocked: '#a371f7',
  unknown: '#8b949e',
};

/**
 * Pairs the two arrays into uPlot's `[xs, ys]`, dropping any point whose position on the
 * time axis is unrepresentable.
 *
 * The pairing has to happen here rather than in the caller: filtering one array without the
 * other would slide every remaining round-trip time onto the wrong moment.
 */
const align = (
  ageSecs: readonly (number | null)[],
  rttMs: readonly (number | null)[],
): [number[], (number | null)[]] => {
  const xs: number[] = [];
  const ys: (number | null)[] = [];
  ageSecs.forEach((age, index) => {
    if (age === null) return;
    xs.push(age);
    ys.push(rttMs[index] ?? null);
  });
  return [xs, ys];
};

interface SparklineProps {
  /** Seconds before now for each point — negative and ascending. */
  readonly ageSecs: readonly (number | null)[];
  /** Round-trip time at each point; `null` wherever the probe did not come back. */
  readonly rttMs: readonly (number | null)[];
  /** Health state, which decides the stroke colour. */
  readonly health: HealthView;
  /** Accessible description; the figures themselves are in the row beside the chart. */
  readonly label: string;
}

/**
 * A dense round-trip-time series.
 *
 * Gaps are gaps: uPlot's default is not to span a `null`, so a stretch of timeouts leaves a
 * break in the line rather than a straight segment drawn through an outage. The x axis is
 * real elapsed time, not a sample index, because probe intervals stretch under backoff.
 *
 * The chart only ever redraws when new data arrives, and Rust emits nothing while the
 * window is hidden — so a minimized app costs no layout and no canvas work at all.
 */
export const Sparkline = ({ ageSecs, rttMs, health, label }: SparklineProps) => {
  const host = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);
  // Read by uPlot's stroke callback at draw time, so a change of state recolours the line
  // without tearing the chart down and building it again.
  const currentHealth = useRef<HealthView>(health);
  currentHealth.current = health;

  useEffect(() => {
    const element = host.current;
    if (!element) return undefined;

    let chart: uPlot | null = null;
    try {
      chart = new uPlot(
        {
          width: WIDTH,
          height: HEIGHT,
          padding: [2, 2, 2, 2],
          legend: { show: false },
          cursor: { show: false },
          scales: { x: { time: false } },
          axes: [{ show: false }, { show: false }],
          series: [
            {},
            {
              stroke: () => STROKE[currentHealth.current],
              width: 1.5,
              points: { show: false },
            },
          ],
        },
        [[], []],
        element,
      );
    } catch {
      // No canvas context — a headless renderer, or a WebView that has lost its surface.
      // The row's numbers carry the same information, so the chart simply does not appear.
      chart = null;
    }
    plot.current = chart;

    return () => {
      chart?.destroy();
      plot.current = null;
    };
  }, []);

  useEffect(() => {
    const chart = plot.current;
    if (!chart) return;

    const [xs, ys] = align(ageSecs, rttMs);
    if (xs.length === 0) return;

    chart.setData([xs, ys]);
  }, [ageSecs, rttMs, health]);

  return <div className="nm-sparkline" ref={host} role="img" aria-label={label} />;
};
