/**
 * How a span of time is written, in the largest unit that still reads as one.
 *
 * Presentation only, like everything in {@link ./format}: Rust supplies the seconds and this
 * decides which words they get. It returns a key and its parameters rather than a string,
 * because every unit on this page is an i18next key — a formatter that concatenated " min"
 * would be the one place a locale could not reach.
 *
 * The thresholds are about readability rather than arithmetic. A monitor is left running for
 * hours on purpose, so "8 h 12 min" has to be sayable; below two minutes the seconds are
 * worth more than the rounding they would lose, which is why 90 seconds reads as itself and
 * not as "2 min".
 */
export interface Span {
  readonly key: 'span.seconds' | 'span.minutes' | 'span.hours';
  readonly params: Readonly<Record<string, number>>;
}

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3_600;

/** Writes a span of seconds as the key and parameters that name it. */
export const spanOf = (secs: number): Span => {
  // A negative span is not a duration; a clock that appeared to move backwards must produce
  // "0 s" rather than a figure with a minus sign in front of it.
  const total = Math.max(0, Math.round(secs));
  if (total < 2 * SECONDS_PER_MINUTE) return { key: 'span.seconds', params: { seconds: total } };
  if (total < SECONDS_PER_HOUR) {
    return { key: 'span.minutes', params: { minutes: Math.round(total / SECONDS_PER_MINUTE) } };
  }
  return {
    key: 'span.hours',
    params: {
      hours: Math.floor(total / SECONDS_PER_HOUR),
      minutes: Math.floor((total % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE),
    },
  };
};
