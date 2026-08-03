import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import { formatBytes, formatCount, formatMs, formatPct, formatRate, NO_VALUE } from './format';

/**
 * Every figure on the page, written down with its unit and in the active locale.
 *
 * `format.ts` decides how many decimal places a number gets; this decides what stands beside
 * it. They are separate because the unit is a **user-visible string** and therefore an
 * i18next key like every other, while the digits are `Intl`'s business — and a helper that
 * concatenated `' ms'` in TypeScript would be the one hardcoded literal in the product.
 *
 * **The dash never takes a unit.** `— ms` claims a millisecond measurement that does not
 * exist; an absent figure is absent, and it says so with nothing after it. That is the same
 * rule `format.ts` keeps against zero, one layer up.
 *
 * Bound once per component rather than passed a locale at every call site: the previous
 * arrangement threaded `i18n.language` through every `<dd>` on the page and still produced
 * `Loss 3` — three per cent or three packets, with nothing on screen to say which.
 */
export interface Figures {
  /** A round trip, a jitter or a pause, in milliseconds. */
  readonly ms: (value: number | null) => string;
  /** A share of something, as a percentage. */
  readonly pct: (value: number | null) => string;
  /** A rate with no unit of its own, for a sentence that supplies the words. */
  readonly rate: (value: number | null) => string;
  /** A volume of traffic, with the largest byte unit that fits. */
  readonly bytes: (value: number | null) => string;
  /** A volume of traffic per second. */
  readonly bytesPerSec: (value: number | null) => string;
  /** A whole count — of processes, of endpoints, of checks. */
  readonly count: (value: number) => string;
}

export const useFigures = (): Figures => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  return useMemo(() => {
    const withUnit = (written: string, key: 'unit.ms' | 'unit.pct' | 'unit.perSecond'): string =>
      written === NO_VALUE ? NO_VALUE : t(key, { value: written });

    return {
      ms: (value) => withUnit(formatMs(value, locale), 'unit.ms'),
      pct: (value) => withUnit(formatPct(value, locale), 'unit.pct'),
      rate: (value) => formatRate(value, locale),
      bytes: (value) => formatBytes(value, locale),
      bytesPerSec: (value) => withUnit(formatBytes(value, locale), 'unit.perSecond'),
      count: (value) => formatCount(value, locale),
    };
  }, [locale, t]);
};
