/**
 * Presentation-only helpers: how a number that Rust already computed is written down.
 *
 * No arithmetic on measurements lives here. Averages, jitter, loss and verdicts are all
 * computed in `nm-core` where they are unit-tested; this module decides how many decimal
 * places they get and what stands in for a value that does not exist.
 *
 * A missing value is always the same dash, never `0`. Rust sends `null` precisely when it
 * has nothing to say, and turning that into a zero here would undo the honesty the whole
 * core is built around.
 */

/** Shown wherever there is no measurement. */
export const NO_VALUE = '—';

type Numeric = number | null;

const formatterFor = (locale: string, digits: number): Intl.NumberFormat =>
  new Intl.NumberFormat(locale, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });

/** A latency or jitter figure in milliseconds, or the dash when there is none. */
export const formatMs = (value: Numeric, locale: string): string =>
  value === null ? NO_VALUE : formatterFor(locale, value < 10 ? 1 : 0).format(value);

/** A loss percentage, or the dash when delivery was never tested. */
export const formatPct = (value: Numeric, locale: string): string =>
  value === null ? NO_VALUE : formatterFor(locale, value > 0 && value < 1 ? 1 : 0).format(value);

/** A whole count, such as how many members of a group are in a state. */
export const formatCount = (value: number, locale: string): string =>
  new Intl.NumberFormat(locale).format(value);
