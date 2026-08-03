import { useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useHelp } from './helpContext';
import type { HelpTopic } from './topics';
import { shortKey, titleKey } from './topics';

interface MetricHelpProps {
  readonly topic: HelpTopic;
}

/**
 * The ⓘ beside a figure: one or two plain sentences, and a way to the rest.
 *
 * The audience is a player who knows their game stutters and does not know what jitter is.
 * Every number on this page therefore has to be able to explain itself in place — a
 * measurement tool that cannot is asking to be trusted on faith, and this audience has no
 * reason to extend that.
 *
 * **Reachable without a mouse, like everything else here.** It is a disclosure button rather
 * than a hover-only tooltip precisely because of what is inside it: a "Learn more" that
 * vanished the moment a keyboard user moved towards it would not be reachable at all. Hover
 * opens it as well, and the pointer can travel into the panel without it closing, because
 * the handlers sit on the wrapper rather than on the button.
 */
export const MetricHelp = ({ topic }: MetricHelpProps) => {
  const { t } = useTranslation();
  const openHelp = useHelp();
  const [pinned, setPinned] = useState(false);
  const [hovered, setHovered] = useState(false);
  const shown = pinned || hovered;

  // Which side the panel hangs from. Anchored left by default; flipped when it would
  // otherwise run past the right edge of the window, which is where every explanation in a
  // right-hand column was being clipped. Measured after layout and before paint, so the
  // panel is never seen in the wrong place, and always from the unflipped position so the
  // decision cannot oscillate.
  const panel = useRef<HTMLSpanElement>(null);
  const [flipped, setFlipped] = useState(false);
  useLayoutEffect(() => {
    if (!shown) {
      setFlipped(false);
      return;
    }
    const box = panel.current?.getBoundingClientRect();
    // A renderer without layout reports every rectangle as zero. Flipping every panel there
    // would be worse than flipping none, so a zero-width measurement decides nothing.
    if (box === undefined || box.width === 0) return;
    setFlipped(box.right > window.innerWidth);
  }, [shown]);

  return (
    <span
      className="nm-help"
      onMouseEnter={() => {
        setHovered(true);
      }}
      onMouseLeave={() => {
        setHovered(false);
      }}
    >
      <button
        type="button"
        className="nm-help__mark"
        aria-expanded={shown}
        aria-label={t('help.explain', { metric: t(titleKey(topic)) })}
        onClick={() => {
          setPinned((current) => !current);
        }}
        onFocus={() => {
          setHovered(true);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') setPinned(false);
        }}
      >
        <span aria-hidden="true">i</span>
      </button>
      {shown && (
        <span
          className={flipped ? 'nm-help__panel nm-help__panel--flipped' : 'nm-help__panel'}
          role="note"
          ref={panel}
        >
          <span className="nm-help__text">{t(shortKey(topic))}</span>
          <button
            type="button"
            className="nm-help__more"
            onClick={() => {
              setPinned(false);
              openHelp(topic);
            }}
            onBlur={() => {
              setHovered(false);
            }}
          >
            {t('help.learnMore')}
          </button>
        </span>
      )}
    </span>
  );
};
