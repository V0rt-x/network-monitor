import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { HelpTopic } from './topics';
import {
  anchorOf,
  bodyKey,
  HELP_SECTIONS,
  paragraphsOf,
  sectionAnchorOf,
  sectionKey,
  shortKey,
  titleKey,
} from './topics';

interface HelpPageProps {
  /** The topic to open at, when the reader arrived from a label's "Learn more". */
  readonly topic: HelpTopic | null;
  /** Where they came from, so they can be put back. `null` when they used the tab. */
  readonly onBack: (() => void) | null;
}

/**
 * What every figure on this page means — as an index, not as a wall.
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
 * **The default is a title and one sentence.** Twenty-eight topics each opening with a full
 * body is about six thousand words with no point of entry, and a reader who followed one
 * "Learn more" had to find their answer inside it. The `short` line was already written for
 * every topic and is already good; the body sits behind *Read more*, which collapses the page
 * to something that can be scanned. Five sections with a contents column and a filter are how
 * it is navigated, and *Back* is how the reader gets out — arriving from a figure's
 * explanation used to be a one-way trip.
 *
 * The first section is why the numbers here are not the ping the game shows. Without it the
 * honest answer looks like a wrong one, and the reader concludes the tool is broken rather
 * than that the number they knew was never what they thought it was.
 */
export const HelpPage = ({ topic, onBack }: HelpPageProps) => {
  const { t } = useTranslation();
  const filterId = useId();
  const [filter, setFilter] = useState('');
  // Which bodies are open. A set rather than one topic: a reader comparing jitter with
  // arrival jitter wants both, and closing one to open the other is the page fighting them.
  const [opened, setOpened] = useState<ReadonlySet<HelpTopic>>(
    () => new Set(topic === null ? [] : [topic]),
  );
  const arrivedAt = useRef<HelpTopic | null>(null);

  // A topic matches on the words a reader has actually seen — its title and its one-line
  // summary. Searching the bodies would match nearly everything on nearly every word.
  const matches = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    return HELP_SECTIONS.map((section) => ({
      id: section.id,
      topics: (section.topics as readonly HelpTopic[]).filter(
        (entry) =>
          needle === '' ||
          t(titleKey(entry)).toLowerCase().includes(needle) ||
          t(shortKey(entry)).toLowerCase().includes(needle),
      ),
    })).filter((section) => section.topics.length > 0);
  }, [filter, t]);

  // Moving focus rather than only scrolling: someone who followed "Learn more" with a
  // keyboard has to land *in* the topic, not merely have it pass by. Once per arrival, so a
  // reader who then scrolls away is not dragged back by an unrelated re-render.
  useEffect(() => {
    if (topic === null || arrivedAt.current === topic) return;
    arrivedAt.current = topic;
    setOpened((current) => new Set(current).add(topic));
    const element = document.getElementById(anchorOf(topic));
    // Scrolling is a nicety and focus is the substance. Guarded because a renderer without
    // layout — the test environment, and any future headless one — has no `scrollIntoView`,
    // and throwing here would leave the reader on a blank page instead of at the answer.
    if (typeof element?.scrollIntoView === 'function') element.scrollIntoView({ block: 'start' });
    element?.focus();
  }, [topic]);

  const toggle = (entry: HelpTopic) => {
    setOpened((current) => {
      const next = new Set(current);
      if (!next.delete(entry)) next.add(entry);
      return next;
    });
  };

  return (
    <div className="nm-help-page">
      <header className="nm-help-page__head">
        {onBack !== null && (
          <button type="button" className="nm-button nm-button--quiet" onClick={onBack}>
            {t('help.back')}
          </button>
        )}
        <h2 className="nm-help-page__title">{t('help.title')}</h2>
        {/* The tab used to be called "What the numbers mean", which is a sentence rather than
            a tab. It is a good subtitle, so nothing was lost by moving it here. */}
        <p className="nm-help-page__subtitle">{t('help.subtitle')}</p>
      </header>

      <p className="nm-help-page__intro">{t('help.intro')}</p>
      {/* Said once, here and in the empty state, rather than two hundred times on the page a
          reader is looking at: an underline is what marks a label as explainable. */}
      <p className="nm-help-page__intro">{t('help.affordance')}</p>

      <div className="nm-field nm-help-page__filter">
        <label htmlFor={filterId}>{t('help.filter')}</label>
        <input
          id={filterId}
          type="search"
          value={filter}
          placeholder={t('help.filterPlaceholder')}
          onChange={(event) => {
            setFilter(event.target.value);
          }}
        />
      </div>

      <div className="nm-help-page__columns">
        <nav className="nm-help-page__contents" aria-label={t('help.contents')}>
          <h3 className="nm-help-page__contentstitle">{t('help.contents')}</h3>
          <ul>
            {matches.map((section) => (
              <li key={section.id}>
                <a href={`#${sectionAnchorOf(section.id)}`}>{t(sectionKey(section.id))}</a>
              </li>
            ))}
          </ul>
        </nav>

        <div className="nm-help-page__topics">
          {matches.length === 0 && <p className="nm-state--pending">{t('help.noMatches')}</p>}

          {matches.map((section) => (
            <section
              key={section.id}
              id={sectionAnchorOf(section.id)}
              className="nm-help-page__group"
            >
              <h3 className="nm-help-page__grouptitle">{t(sectionKey(section.id))}</h3>

              {section.topics.map((entry) => (
                <article
                  key={entry}
                  id={anchorOf(entry)}
                  className={
                    entry === topic
                      ? 'nm-help-page__section nm-help-page__section--opened'
                      : 'nm-help-page__section'
                  }
                  // Focusable so that arriving at a topic puts the reader inside it, and only
                  // by script — it is not a control, so it must not be in the tab order.
                  tabIndex={-1}
                >
                  <h4 className="nm-help-page__heading">{t(titleKey(entry))}</h4>
                  <p className="nm-help-page__short">{t(shortKey(entry))}</p>
                  <button
                    type="button"
                    className="nm-help-page__more"
                    aria-expanded={opened.has(entry)}
                    onClick={() => {
                      toggle(entry);
                    }}
                  >
                    {opened.has(entry) ? t('help.readLess') : t('help.readMore')}
                  </button>
                  {/* One `<p>` per paragraph the author wrote. Inside a single `<p>` a blank
                      line collapses to a space, which shipped the longest topic as one
                      unbroken ~350-word block. */}
                  {opened.has(entry) &&
                    paragraphsOf(t(bodyKey(entry))).map((paragraph) => (
                      <p key={paragraph} className="nm-help-page__body">
                        {paragraph}
                      </p>
                    ))}
                </article>
              ))}
            </section>
          ))}
        </div>
      </div>
    </div>
  );
};
