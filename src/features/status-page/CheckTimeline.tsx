import { useTranslation } from 'react-i18next';

import type { CheckView } from '../../shared/ipc';
import { checkMarkKey, checkModifier } from './labels';

interface CheckTimelineProps {
  readonly checks: readonly CheckView[];
  readonly label: string;
}

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
 */
export const CheckTimeline = ({ checks, label }: CheckTimelineProps) => {
  const { t } = useTranslation();

  if (checks.length === 0) {
    return <p className="nm-timeline__empty">{t('status.noChecks')}</p>;
  }

  return (
    <ol className="nm-timeline" aria-label={label}>
      {checks.map((check, index) => {
        const word = t(checkMarkKey(check.mark));
        return (
          <li
            // A check has no identity of its own; what it has is a position and a moment,
            // and the pair is unique within a strip.
            key={`${String(index)}:${String(check.ageSecs ?? 0)}`}
            className={`nm-check ${checkModifier(check.mark)}`}
            title={word}
          >
            <span className="nm-visually-hidden">{word}</span>
          </li>
        );
      })}
    </ol>
  );
};
