import { useTranslation } from 'react-i18next';

import { useHelp } from '../help/helpContext';

/**
 * Why the figures here are not the ping the game shows.
 *
 * **The single most important string in the application.** An endpoint that answers nothing
 * we can send is the normal state of a game's match server — nothing listens on a game port
 * but the game — so the row that matters most is the one whose round trip is a dash. Without
 * this the honest answer looks like a wrong one, and the reader concludes the tool is broken
 * rather than that the number they knew was never what they thought it was.
 *
 * Collapsed to one line, because it is the same explanation every time and the reader who has
 * already had it does not need it again; expanded it makes three points, and the bundled help
 * makes them at length in its first section.
 */
export const WhyNotYourPing = () => {
  const { t } = useTranslation();
  const openHelp = useHelp();

  return (
    <details className="nm-whyping">
      <summary>{t('apps.whyPing.summary')}</summary>
      <ul className="nm-whyping__points">
        <li>{t('apps.whyPing.timed')}</li>
        <li>{t('apps.whyPing.refused')}</li>
        <li>{t('apps.whyPing.instead')}</li>
      </ul>
      <button
        type="button"
        className="nm-help__more"
        onClick={() => {
          openHelp('ping');
        }}
      >
        {t('help.learnMore')}
      </button>
    </details>
  );
};
