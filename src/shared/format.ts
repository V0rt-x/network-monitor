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

/**
 * Units for a byte count, largest first.
 *
 * Decimal rather than binary, because these are network volumes and every other tool the
 * user compares against — the router's page, the ISP's meter — counts them the same way.
 */
const BYTE_UNITS = [
  { limit: 1e9, divisor: 1e9, suffix: 'GB' },
  { limit: 1e6, divisor: 1e6, suffix: 'MB' },
  { limit: 1e3, divisor: 1e3, suffix: 'kB' },
] as const;

/**
 * A volume of traffic, or the dash when nothing counted it.
 *
 * `null` is the answer wherever the platform has no byte counters at all, and it must stay
 * visibly different from a measured zero: a busy game reported as "0 B" would be a lie the
 * user cannot see through.
 *
 * The unit is a symbol rather than a translated word — `kB` and `MB` are written the same
 * in every locale this app targets — while the number itself goes through `Intl`.
 */
export const formatBytes = (value: Numeric, locale: string): string => {
  if (value === null) return NO_VALUE;

  const unit = BYTE_UNITS.find((entry) => value >= entry.limit);
  if (!unit) return `${new Intl.NumberFormat(locale).format(value)} B`;

  const scaled = value / unit.divisor;
  return `${formatterFor(locale, scaled < 10 ? 1 : 0).format(scaled)} ${unit.suffix}`;
};
