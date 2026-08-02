import type { TransportView } from '../../shared/ipc';

/** One line on an application's chart. */
export interface ChartLine {
  /** The endpoint it belongs to. Two lines can share this — see `isPath`. */
  readonly endpoint: string;
  /**
   * How the application reaches the endpoint.
   *
   * The chart follows the page's emphasis: the match traffic is drawn at full weight and the
   * supporting connections lighter. It is emphasis and not a verdict — the worst-first list
   * remains the only authority on health.
   */
  readonly transport: TransportView;
  /** What the line is called in the chart's own legend. */
  readonly label: string;
  /** Milliseconds in each slot; `null` is a gap and is never drawn across. */
  readonly values: readonly (number | null)[];
  /** The colour the endpoint is identified by. */
  readonly colour: string;
  /**
   * Whether this is the route to the endpoint rather than the endpoint itself.
   *
   * Drawn dashed, and it must never read as a round trip to the server: it belongs to a
   * router short of it, at a distance nothing measured.
   */
  readonly isPath: boolean;
}

/**
 * A tick label on the chart's logarithmic round-trip axis.
 *
 * uPlot labels a log scale in exponents by default; whole milliseconds are what someone
 * comparing latencies actually reads. Sub-millisecond ticks keep one decimal, which is the
 * only place a round trip on this chart is ever that small.
 *
 * **A split can be `null`.** A log axis runs its splits through a filter that blanks the
 * minor ticks between powers, and the filtered array is what reaches here. Treating one of
 * those as a number throws in the middle of a draw — which leaves an empty canvas, no error
 * anywhere, and a note underneath still describing a chart that is not there. It cost an
 * afternoon once; the test beside this exists so it cannot cost another.
 */
export const formatAxisMs = (value: number | null | undefined): string => {
  if (value === null || value === undefined) return '';
  if (!Number.isFinite(value)) return '';
  return value >= 1 ? String(Math.round(value)) : value.toFixed(1);
};

/**
 * A tick label on the chart's time axis.
 *
 * The axis runs from where monitoring began, so the values are elapsed seconds rather than
 * ages: `0:00` is the moment the user started watching this application, and the drawing
 * grows rightwards from it. Minutes and seconds because two minutes of history read as
 * `1:30` far more readily than as `90`.
 *
 * **Hours appear as hours.** Found by running the build against a session that had been
 * watching a game for eleven hours: the axis read `652:10`, which is arithmetically the truth
 * and unreadable as a time. A monitor is left running for exactly that long — that is what it
 * is for — so anything past an hour reads `10:52:10`.
 *
 * A blanked split arrives as `null`, exactly as on the round-trip axis, and treating one as a
 * number throws in the middle of a draw — which leaves an empty canvas and no error anywhere.
 */
export const formatAxisElapsed = (value: number | null | undefined): string => {
  if (value === null || value === undefined) return '';
  if (!Number.isFinite(value)) return '';
  const total = Math.max(0, Math.round(value));
  const seconds = total % 60;
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3_600);
  const tail = `${String(minutes).padStart(hours > 0 ? 2 : 1, '0')}:${String(seconds).padStart(2, '0')}`;
  return hours > 0 ? `${String(hours)}:${tail}` : tail;
};

/**
 * The smallest round trip the chart's logarithmic axis can place, in milliseconds.
 *
 * A logarithmic scale has no zero, so a measurement of zero — which needs a round trip
 * faster than a microsecond and cannot happen over a network — would break the axis rather
 * than appear on it. Anything at or below this is drawn *at* the floor instead. That is a
 * drawing decision and not a measurement: the exact figure is in the row beside the chart,
 * where it is never rounded.
 */
export const LOG_FLOOR_MS = 0.01;

/**
 * Pairs the shared time axis with every line, in the layout uPlot wants.
 *
 * Rust has already placed every endpoint's samples on one grid, so the arrays line up by
 * construction. What is left is the one hazard of the wire format: a float that could not be
 * represented crosses as `null`, and dropping such a slot from the axis without dropping it
 * from *every* series would slide the remaining measurements onto the wrong moments. Doing
 * it here, once, is what guarantees they are dropped together.
 *
 * A `null` inside a series is untouched — that is a gap, and uPlot does not draw across one,
 * which is how an outage stays a break in the line rather than a straight segment through it.
 */
export const alignSeries = (
  elapsedSecs: readonly (number | null)[],
  lines: readonly ChartLine[],
): (number | null)[][] => {
  const xs: number[] = [];
  const ys: (number | null)[][] = lines.map(() => []);
  elapsedSecs.forEach((elapsed, slot) => {
    if (elapsed === null) return;
    xs.push(elapsed);
    lines.forEach((line, index) => {
      const value = line.values[slot] ?? null;
      ys[index]?.push(value === null ? null : Math.max(value, LOG_FLOOR_MS));
    });
  });
  return [xs, ...ys];
};
