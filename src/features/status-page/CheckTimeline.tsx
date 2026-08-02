import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CheckView } from '../../shared/ipc';
import { checkMarkKey, checkModifier } from './labels';

interface CheckTimelineProps {
  readonly checks: readonly CheckView[];
  readonly label: string;
}

/**
 * How long ago a check happened, in the largest unit that still reads as a time.
 *
 * A full strip reaches a quarter of an hour back, so seconds all the way would print
 * "1035 s ago" at the left-hand edge — arithmetically right and useless. Two minutes is
 * where the rounding stops costing more than the shorter unit buys: below it a check is
 * "90 s ago" rather than the "2 min ago" that rounding would produce.
 */
const agedKey = (secs: number) =>
  secs < 120
    ? ({ key: 'status.timeline.agedSeconds', params: { seconds: Math.round(secs) } } as const)
    : ({ key: 'status.timeline.agedMinutes', params: { minutes: Math.round(secs / 60) } } as const);

/**
 * The recent checks of one endpoint, oldest on the left.
 *
 * One cell is one check and nothing more — a fact, not a verdict. That is what makes the
 * strip worth the space beside a card: it is the evidence the headline was reached from, so
 * a user can see the difference between "one packet went missing twenty minutes ago" and
 * "nothing has answered since". It also keeps the four ways a check can fail apart, which a
 * single colour on the card cannot.
 *
 * Deliberately not a chart. There is no round-trip axis here: a status page asks whether a
 * service answers, and the latency figure sits beside the strip where it belongs.
 *
 * **A cell says when it happened, by pointer and by keyboard alike.** The strip is one tab
 * stop with arrow keys inside it rather than a tab stop per cell: a page of thirteen
 * services would otherwise put several hundred stops between a keyboard user and the next
 * thing they wanted. What the pointer and the arrow keys reach is the same readout, written
 * out under the strip rather than hidden in a tooltip — a `title` attribute is unreachable
 * without a mouse, which is the failure this replaces.
 *
 * **Colour is never the only channel.** Every cell carries its translated word for a screen
 * reader whatever it is being pointed at, and the page's legend names the five states in
 * words beside their colours.
 */
export const CheckTimeline = ({ checks, label }: CheckTimelineProps) => {
  const { t } = useTranslation();
  const [reading, setReading] = useState<number | null>(null);

  if (checks.length === 0) {
    return <p className="nm-timeline__empty">{t('status.noChecks')}</p>;
  }

  const step = (by: number) => {
    setReading((current) => {
      const next = (current ?? checks.length - 1) + by;
      return Math.min(Math.max(next, 0), checks.length - 1);
    });
  };

  const at = reading === null ? undefined : checks[reading];
  // The position and the state are always sayable; the age is not, and it is never guessed
  // at. A check whose moment did not come through is read out without one rather than with
  // an invented "0 s ago".
  const aged =
    at?.ageSecs === null || at?.ageSecs === undefined ? null : agedKey(Math.abs(at.ageSecs));
  const readout =
    at === undefined
      ? t('status.timeline.readoutIdle')
      : [
          t('status.timeline.position', { index: (reading ?? 0) + 1, count: checks.length }),
          aged === null
            ? t(checkMarkKey(at.mark))
            : t('status.timeline.cell', {
                state: t(checkMarkKey(at.mark)),
                age: t(aged.key, aged.params),
              }),
        ].join(' · ');

  return (
    <div className="nm-timeline__strip">
      <ol
        className="nm-timeline"
        aria-label={label}
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === 'ArrowLeft') step(-1);
          else if (event.key === 'ArrowRight') step(1);
          else if (event.key === 'Home') setReading(0);
          else if (event.key === 'End') setReading(checks.length - 1);
          else return;
          // Only for the keys the strip handled: the page must still scroll on the rest.
          event.preventDefault();
        }}
        onBlur={() => {
          setReading(null);
        }}
        onMouseLeave={() => {
          setReading(null);
        }}
      >
        {checks.map((check, index) => {
          const word = t(checkMarkKey(check.mark));
          return (
            <li
              // A check has no identity of its own; what it has is a position and a moment,
              // and the pair is unique within a strip.
              key={`${String(index)}:${String(check.ageSecs)}`}
              className={`nm-check ${checkModifier(check.mark)}${
                index === reading ? ' nm-check--read' : ''
              }`}
              onMouseEnter={() => {
                setReading(index);
              }}
            >
              <span className="nm-visually-hidden">{word}</span>
            </li>
          );
        })}
      </ol>
      {/* Polite rather than assertive: a reading the user asked for by pointing, not an
          announcement that should interrupt whatever they were being told. */}
      <p className="nm-timeline__readout" role="status">
        {readout}
      </p>
    </div>
  );
};
