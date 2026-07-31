/** One line on an application's chart. */
export interface ChartLine {
  /** The endpoint it belongs to. Two lines can share this — see `isPath`. */
  readonly endpoint: string;
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
  ageSecs: readonly (number | null)[],
  lines: readonly ChartLine[],
): (number | null)[][] => {
  const xs: number[] = [];
  const ys: (number | null)[][] = lines.map(() => []);
  ageSecs.forEach((age, slot) => {
    if (age === null) return;
    xs.push(age);
    lines.forEach((line, index) => {
      ys[index]?.push(line.values[slot] ?? null);
    });
  });
  return [xs, ...ys];
};
