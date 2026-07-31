import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { DashboardPage } from './features/dashboard/DashboardPage';
import { SettingsPage } from './features/settings/SettingsPage';
import { useTrayLabels } from './features/shell/useTrayLabels';
import { hideToTray, quitApp } from './shared/ipc';

/** The two pages Phase 3 ships. A router would be a dependency for one boolean. */
type Page = 'dashboard' | 'settings';

/** Literal keys, so a rename in `common.json` is a compile error rather than a blank tab. */
const PAGES = [
  { id: 'dashboard', labelKey: 'nav.dashboard' },
  { id: 'settings', labelKey: 'nav.settings' },
] as const satisfies readonly { readonly id: Page; readonly labelKey: string }[];

export const App = () => {
  const { t } = useTranslation();
  const [page, setPage] = useState<Page>('dashboard');
  useTrayLabels();

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
        {page === 'dashboard' ? <DashboardPage /> : <SettingsPage />}
      </main>
    </div>
  );
};
