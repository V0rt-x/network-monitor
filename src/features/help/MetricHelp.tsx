import type { ReactNode } from 'react';
import { useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useHelp } from './helpContext';
import type { HelpTopic } from './topics';
import { shortKey, titleKey } from './topics';

interface MetricHelpProps {
  readonly topic: HelpTopic;
  /**
   * The label being explained.
   *
   * Omitted where the topic's own title *is* the label, which is the ordinary case: a column
   * heading reading "Ping (RTT)" is explained by the `rtt` topic, whose title is the same
   * words, so repeating them at the call site is a second place to keep them in step.
   */
  readonly children?: ReactNode;
}

/**
 * A label that explains itself, in one or two plain sentences, with a way to the rest.
 *
 * The audience is a player who knows their game stutters and does not know what jitter is.
 * Every figure on the page therefore has to be able to explain itself in place — a
 * measurement tool that cannot is asking to be trusted on faith, and this audience has no
 * reason to extend that.
 *
 * **The label carries the explanation; there is no separate mark.** It used to draw an ⓘ
 * beside the word, which is a second target that names nothing next to a word that already
 * names the quantity. The rule this implements was written in the singular — "an ⓘ on every
 * figure" — and silently assumed one figure on screen; on an application with twenty
 * connections it produced up to two hundred and sixty identical marks, and a mark repeated
 * two hundred times does not explain a figure, it hides it. A hairline underline says the same
 * thing, costs no width, and creates no second stop for a keyboard — solid rather than
 * dotted, because a dotted rule under every heading reads as a page of links from 1998 in a
 * product that has no links at all.
 *
 * That the underline means something is stated once, in the help's own introduction and in
 * the empty state — once, rather than two hundred times.
 *
 * **Reachable without a mouse, like everything else here.** It is a disclosure button rather
 * than a hover-only tooltip precisely because of what is inside it: a "Learn more" that
 * vanished the moment a keyboard user moved towards it would not be reachable at all. Hover
 * opens it as well, and the pointer can travel into the panel without it closing, because
 * the handlers sit on the wrapper rather than on the button.
 */
export const MetricHelp = ({ topic, children }: MetricHelpProps) => {
  const { t } = useTranslation();
  const openHelp = useHelp();
  const [pinned, setPinned] = useState(false);
  const [hovered, setHovered] = useState(false);
  const shown = pinned || hovered;

  // Which corner the panel hangs from. Anchored below-left by default; flipped on either
  // axis when it would otherwise leave the window — the right edge is where every
  // explanation in a right-hand column was being clipped, and the bottom edge is where one
  // opened on the last row of a long table went. Measured after layout and before paint, so
  // the panel is never seen in the wrong place, and always from the unflipped position so
  // the decision cannot oscillate.
  const panel = useRef<HTMLSpanElement>(null);
  const [flipped, setFlipped] = useState(false);
  const [above, setAbove] = useState(false);
  useLayoutEffect(() => {
    if (!shown) {
      setFlipped(false);
      setAbove(false);
      return;
    }
    const box = panel.current?.getBoundingClientRect();
    // A renderer without layout reports every rectangle as zero. Flipping every panel there
    // would be worse than flipping none, so a zero-width measurement decides nothing.
    if (box === undefined || box.width === 0) return;
    setFlipped(box.right > window.innerWidth);
    // Only when there is somewhere to go: a panel taller than the window would flip upwards
    // and leave the *top* instead, which trades one clipped sentence for another.
    setAbove(box.bottom > window.innerHeight && box.height < box.top);
  }, [shown]);

  const panelClass = [
    'nm-help__panel',
    flipped ? 'nm-help__panel--flipped' : '',
    above ? 'nm-help__panel--above' : '',
  ]
    .filter(Boolean)
    .join(' ');

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
        className="nm-explains"
        aria-expanded={shown}
        // The accessible name is the same sentence the ⓘ used to carry, so nothing a screen
        // reader hears has changed: the word alone would be indistinguishable from a
        // heading, and what this control does is explain it.
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
        {children ?? t(titleKey(topic))}
      </button>
      {shown && (
        <span className={panelClass} role="note" ref={panel}>
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
