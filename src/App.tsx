import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { AppMonitorPage } from './features/app-monitor/AppMonitorPage';
import { HelpProvider } from './features/help/HelpProvider';
import { HelpPage } from './features/help/HelpPage';
import type { HelpTopic } from './features/help/topics';
import { NetworkPage } from './features/network/NetworkPage';
import { SettingsPage } from './features/settings/SettingsPage';
import { useTrayLabels } from './features/shell/useTrayLabels';

/**
 * The pages the app ships. A router would be a dependency for one switch.
 *
 * Four, not five. The dashboard and the service status page answered one question between
 * them — is it me, the border, or that service — and answering it meant switching tabs
 * while holding one page in your head to read the other; they are now the two halves of
 * *Network*.
 */
type Page = 'network' | 'apps' | 'help' | 'settings';

/** Literal keys, so a rename in `common.json` is a compile error rather than a blank tab. */
const PAGES = [
  { id: 'network', labelKey: 'nav.network' },
  { id: 'apps', labelKey: 'nav.apps' },
  { id: 'help', labelKey: 'nav.help' },
  { id: 'settings', labelKey: 'nav.settings' },
] as const satisfies readonly { readonly id: Page; readonly labelKey: string }[];

export const App = () => {
  const { t } = useTranslation();
  const [page, setPage] = useState<Page>('network');
  // Which topic the help opens at, when the reader arrived from a label's own explanation.
  const [topic, setTopic] = useState<HelpTopic | null>(null);
  // And where they came from, so "Back" puts them where they were. Following a "Learn more"
  // used to be a one-way trip: the only way out was the tab, which loses your place.
  const [cameFrom, setCameFrom] = useState<Page | null>(null);
  useTrayLabels();

  /**
   * Which page is mounted.
   *
   * Only one at a time, deliberately: an unmounted page holds no chart and receives no
   * events, so the hidden ones cost nothing to keep in the app.
   */
  const pageFor = (current: Page) => {
    switch (current) {
      case 'network':
        return <NetworkPage />;
      case 'apps':
        return <AppMonitorPage />;
      case 'help':
        return (
          <HelpPage
            topic={topic}
            onBack={
              cameFrom === null
                ? null
                : () => {
                    setPage(cameFrom);
                    setCameFrom(null);
                    setTopic(null);
                  }
            }
          />
        );
      case 'settings':
        return <SettingsPage />;
    }
  };

  return (
    <div className="nm-app">
      {/* The title and the navigation, and nothing else.
          There were two buttons in a third column: `Minimize to tray`, which duplicated what
          the window's own close button already does, and — before 6.7 moved it to Settings —
          `Quit` beside it, so two adjacent controls in the corner of every screen hid the
          window and ended the monitoring. Both are gone. The two exits left are the ones a
          desktop user already knows: the window closes to the tray, and the tray quits. */}
      <header className="nm-app__header">
        <h1 className="nm-app__title">{t('app.name')}</h1>

        <nav className="nm-nav" aria-label={t('nav.label')}>
          {PAGES.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={page === entry.id ? 'nm-nav__tab nm-nav__tab--active' : 'nm-nav__tab'}
              aria-current={page === entry.id ? 'page' : undefined}
              onClick={() => {
                // Reached from the tab rather than from a label, so it opens at the top and
                // offers no way back: whatever topic was last asked for is not what this
                // click meant, and there is nowhere the reader was taken away from.
                if (entry.id === 'help') {
                  setTopic(null);
                  setCameFrom(null);
                }
                setPage(entry.id);
              }}
            >
              {t(entry.labelKey)}
            </button>
          ))}
        </nav>
      </header>

      <main className="nm-app__body">
        <HelpProvider
          openHelp={(entry) => {
            setTopic(entry);
            setCameFrom(page === 'help' ? cameFrom : page);
            setPage('help');
          }}
        >
          {pageFor(page)}
        </HelpProvider>
      </main>
    </div>
  );
};
