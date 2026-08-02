import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { AppMonitorPage } from './features/app-monitor/AppMonitorPage';
import { HelpProvider } from './features/help/HelpProvider';
import { HelpPage } from './features/help/HelpPage';
import type { HelpTopic } from './features/help/topics';
import { NetworkPage } from './features/network/NetworkPage';
import { SettingsPage } from './features/settings/SettingsPage';
import { useTrayLabels } from './features/shell/useTrayLabels';
import { hideToTray, quitApp } from './shared/ipc';

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
  // Which section the help opens at, when the reader arrived from a metric's own ⓘ.
  const [topic, setTopic] = useState<HelpTopic | null>(null);
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
        return <HelpPage topic={topic} />;
      case 'settings':
        return <SettingsPage />;
    }
  };

  return (
    <div className="nm-app">
      <header className="nm-app__header">
        <div>
          <h1 className="nm-app__title">{t('app.name')}</h1>
          <p className="nm-app__tagline">{t('app.tagline')}</p>
        </div>

        <nav className="nm-nav" aria-label={t('nav.label')}>
          {PAGES.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={page === entry.id ? 'nm-nav__tab nm-nav__tab--active' : 'nm-nav__tab'}
              aria-current={page === entry.id ? 'page' : undefined}
              onClick={() => {
                // Reached from the tab rather than from a metric, so it opens at the top:
                // whatever section was last asked for is not what this click meant.
                if (entry.id === 'help') setTopic(null);
                setPage(entry.id);
              }}
            >
              {t(entry.labelKey)}
            </button>
          ))}
        </nav>

        <div className="nm-app__actions">
          {/* Both stay reachable from the window: the tray menu only exists once the
              translated labels have reached Rust, and a user must never be stuck. */}
          <button
            type="button"
            className="nm-button"
            onClick={() => {
              void hideToTray();
            }}
          >
            {t('app.minimize')}
          </button>
          <button
            type="button"
            className="nm-button nm-button--quiet"
            onClick={() => {
              void quitApp();
            }}
          >
            {t('app.quit')}
          </button>
        </div>
      </header>

      <main className="nm-app__body">
        <HelpProvider
          openHelp={(entry) => {
            setTopic(entry);
            setPage('help');
          }}
        >
          {pageFor(page)}
        </HelpProvider>
      </main>
    </div>
  );
};
