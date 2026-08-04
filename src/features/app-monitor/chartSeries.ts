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
 * A tick label on the chart's time axis: the time of day, to the second.
 *
 * It used to read elapsed time — `1:30` for a slot ninety seconds after monitoring began.
 * That answers "how long ago", and the reader's question after a stutter is "**was that when
 * it happened**", which needs a clock. No date, because the axis never spans one; seconds,
 * because a three-second slot cannot be placed without them.
 *
 * `epochMs` is the wall-clock moment of elapsed zero, sent by Rust and recomputed on every
 * emission as the wall clock minus the *monotonic* elapsed. So the measurement never leaves
 * the monotonic clock — a system clock adjusted mid-session moves every label together and
 * moves no sample relative to its neighbours — and the wall clock is used for display only,
 * which is exactly what `CLAUDE.md` permits.
 *
 * A blanked split arrives as `null`, exactly as on the round-trip axis, and treating one as a
 * number throws in the middle of a draw — which leaves an empty canvas and no error anywhere.
 */
export const formatClock = (
  elapsedSecs: number | null | undefined,
  epochMs: number | null,
  locale: string,
): string => {
  if (elapsedSecs === null || elapsedSecs === undefined || epochMs === null) return '';
  if (!Number.isFinite(elapsedSecs) || !Number.isFinite(epochMs)) return '';
  const at = new Date(epochMs + elapsedSecs * 1_000);
  if (Number.isNaN(at.getTime())) return '';
  return new Intl.DateTimeFormat(locale, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(at);
};

/**
 * A span of the chart, as a duration a reader can be told.
 *
 * Level two states what the view covers and at what resolution; this is the first half of
 * that sentence. Whole minutes past a minute, because a span is chosen by dragging and no
 * reader cares that it landed on 19 min 43 s.
 */
export const formatSpan = (seconds: number): string => {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) return `${String(total)} s`;
  const minutes = Math.round(total / 60);
  if (minutes < 60) return `${String(minutes)} min`;
  return `${String(Math.floor(minutes / 60))} h ${String(minutes % 60)} min`;
};

/**
 * Joins the fetched history to the pushed window, by slot.
 *
 * Rust decides where a slot begins and what is in it; this concatenates two arrays that are
 * already on the same ladder, which is not a decision about the numbers. The seam is the
 * first elapsed value the live window carries: everything strictly before it comes from the
 * history, everything from it onwards from the live window, so a slot they both hold is taken
 * from the live one and never drawn twice.
 *
 * **A gap the UI created would be a fabricated loss.** A break in this chart means packets
 * that did not come back, so the backfill matters more than convenience: while the window was
 * hidden Rust kept measuring and emitted nothing, and this is what closes the hole that left.
 */
export const stitchAxis = (
  history: readonly (number | null)[],
  live: readonly (number | null)[],
): { elapsedSecs: (number | null)[]; fromHistory: number } => {
  const seam = live.find((value) => value !== null) ?? Number.POSITIVE_INFINITY;
  const before = history.filter((elapsed): elapsed is number => elapsed !== null && elapsed < seam);
  return { elapsedSecs: [...before, ...live], fromHistory: before.length };
};

/** One line's values on a stitched axis, taking the live window wherever the two overlap. */
export const stitchValues = (
  history: readonly (number | null)[],
  fromHistory: number,
  live: readonly (number | null)[],
): (number | null)[] => [
  // A history shorter than its own axis pads with gaps rather than sliding: every array on
  // this axis has to be the same length or the samples land on the wrong moments.
  ...Array.from({ length: fromHistory }, (_, index) => history[index] ?? null),
  ...live,
];

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
