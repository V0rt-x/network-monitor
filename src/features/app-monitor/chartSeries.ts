import type { TransportView } from '../../shared/ipc';

/**
 * The identifier of an endpoint's row, so a selection on the chart can reach it.
 *
 * Scoped by application because two cards can be watching the same address, and scrolling to
 * whichever one happened to render first would take the reader to another game's card.
 */
export const rowIdOf = (app: number, endpoint: string) =>
  `nm-endpoint-${String(app)}-${endpoint}` as const;

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
  /**
   * The endpoint's address, plain.
   *
   * Beside `label`, which for a route reads "Route to …". The tooltip names the address and
   * names the quantity separately, so a route entry reads "… · route 91 ms" rather than
   * "Route to … · route 91 ms" — the same fact stated twice.
   */
  readonly address: string;
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

/** One line's value at one moment, as the tooltip and the spoken readout state it. */
export interface ChartReadingEntry {
  /** The endpoint the line belongs to, so selecting the entry can pin it. */
  readonly endpoint: string;
  /** The endpoint's address, which is what the entry is named by. */
  readonly address: string;
  readonly colour: string;
  /**
   * Whether this is the route to the endpoint rather than the endpoint itself.
   *
   * The never-merge rule, applied to the tooltip: an entry for a route says *route*, and the
   * word *ping* appears on no route entry in any form.
   */
  readonly isPath: boolean;
  /** Milliseconds, or `null` where nothing came back in that slot. */
  readonly valueMs: number | null;
}

/** Every line at one moment. */
export interface ChartReading {
  /** Seconds since monitoring began, for the moment being read. */
  readonly elapsedSecs: number | null;
  readonly entries: readonly ChartReadingEntry[];
}

/**
 * What every line was doing at one slot.
 *
 * **Every line, not the nearest one.** The chart's stated job is "which of these is the odd
 * one out", and at a given second that is a question about all of them at once — so the
 * tooltip is the legend the chart never had, rather than a label for whatever the pointer
 * happened to be near.
 *
 * Worst first, which is the order the list beside the chart is already in, so the two agree.
 * A line with nothing in that slot sorts last and keeps its `null`: it is read out as *no
 * reply*, never as `0` and never by being quietly left out of the list, because a silently
 * missing entry is indistinguishable from a line that is doing fine.
 */
export const readingAt = (
  aligned: readonly (readonly (number | null)[])[],
  lines: readonly ChartLine[],
  slot: number,
): ChartReading => {
  const entries = lines.map((line, index) => ({
    endpoint: line.endpoint,
    address: line.address,
    colour: line.colour,
    isPath: line.isPath,
    valueMs: aligned[index + 1]?.[slot] ?? null,
  }));
  entries.sort((left, right) => {
    if (left.valueMs === null && right.valueMs === null) return 0;
    if (left.valueMs === null) return 1;
    if (right.valueMs === null) return -1;
    return right.valueMs - left.valueMs;
  });
  return { elapsedSecs: aligned[0]?.[slot] ?? null, entries };
};

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
