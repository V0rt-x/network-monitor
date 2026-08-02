import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import type { HelpTopic } from './topics';
import { anchorOf, bodyKey, HELP_TOPICS, titleKey } from './topics';

interface HelpPageProps {
  /** The section to open at, when the reader arrived from a metric's "Learn more". */
  readonly topic: HelpTopic | null;
}

/**
 * What every figure on this page means, in plain sentences.
 *
 * **Bundled, not a website.** An external link is a network request this product promised
 * never to make on the user's behalf, and it is useless to someone who is being filtered —
 * which is the audience. So the explanations ship inside the application and are readable
 * with no connection at all.
 *
 * No link ships here, and that is a decision rather than an omission. Opening one in the
 * system browser is the only honest way to follow it — a link that navigated the window away
 * would leave the user with no application — and doing that means a Tauri plugin. Adding a
 * dependency for zero links is not a trade worth making; it becomes one the first time a
 * link earns its place.
 *
 * The first section is why the numbers here are not the ping the game shows. Without it the
 * honest answer looks like a wrong one, and the reader concludes the tool is broken rather
 * than that the number they knew was never what they thought it was.
 */
export const HelpPage = ({ topic }: HelpPageProps) => {
  const { t } = useTranslation();
  const heading = useRef<HTMLHeadingElement>(null);

  // Moving focus rather than only scrolling: someone who followed "Learn more" with a
  // keyboard has to land *in* the section, not merely have it pass by.
  useEffect(() => {
    if (topic === null) return;
    const element = document.getElementById(anchorOf(topic));
    // Scrolling is a nicety and focus is the substance. Guarded because a renderer without
    // layout — the test environment, and any future headless one — has no `scrollIntoView`,
    // and throwing here would leave the reader on a blank page instead of at the answer.
    if (typeof element?.scrollIntoView === 'function') element.scrollIntoView({ block: 'start' });
    element?.focus();
  }, [topic]);

  return (
    <div className="nm-help-page">
      <h2 className="nm-help-page__title" ref={heading}>
        {t('help.title')}
      </h2>
      <p className="nm-help-page__intro">{t('help.intro')}</p>

      {HELP_TOPICS.map((entry) => (
        <section
          key={entry}
          id={anchorOf(entry)}
          className={
            entry === topic
              ? 'nm-help-page__section nm-help-page__section--opened'
              : 'nm-help-page__section'
          }
          // Focusable so that arriving at a section puts the reader inside it, and only
          // by script — it is not a control, so it must not be in the tab order.
          tabIndex={-1}
        >
          <h3 className="nm-help-page__heading">{t(titleKey(entry))}</h3>
          <p className="nm-help-page__body">{t(bodyKey(entry))}</p>
        </section>
      ))}
    </div>
  );
};
