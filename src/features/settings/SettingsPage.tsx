import { useTranslation } from 'react-i18next';

import { quitApp } from '../../shared/ipc';
import { settingsProblemKey } from '../dashboard/labels';
import { useSettings } from './useSettings';

/**
 * Everything the user can configure, and nothing that guesses on their behalf.
 *
 * The country list comes from Rust — it is exactly the set of bundled baseline lists — and
 * there is no geo-detection anywhere in the product: working out where someone is means
 * asking a remote service, which this application never does.
 */
export const SettingsPage = () => {
  const { t, i18n } = useTranslation();
  const { state, change } = useSettings();

  if (state.kind === 'loading') {
    return <p className="nm-state--pending">{t('settings.loading')}</p>;
  }

  if (state.kind === 'unavailable') {
    return (
      <p className="nm-state--degraded" role="alert">
        {t('settings.unavailable')}
      </p>
    );
  }

  const { settings, countries, minIntervalSecs, maxIntervalSecs, problem, networkSnapshot } =
    state.view;
  const countryName = new Intl.DisplayNames([i18n.language], { type: 'region' });

  return (
    <section className="nm-settings">
      {problem !== null && (
        <p className="nm-state--degraded" role="alert">
          {t(settingsProblemKey(problem))}
        </p>
      )}

      <div className="nm-field">
        <label htmlFor="nm-language">{t('settings.language')}</label>
        <select
          id="nm-language"
          value={settings.language}
          onChange={(event) => {
            change({ language: event.target.value });
          }}
        >
          <option value="en">{t('settings.languageName.en')}</option>
        </select>
        <p className="nm-field__hint">{t('settings.languageHint')}</p>
      </div>

      <div className="nm-field">
        <label htmlFor="nm-country">{t('settings.country')}</label>
        <select
          id="nm-country"
          value={settings.country}
          onChange={(event) => {
            change({ country: event.target.value });
          }}
        >
          {countries.map((code) => (
            <option key={code} value={code}>
              {countryName.of(code.toUpperCase()) ?? code}
            </option>
          ))}
        </select>
        <p className="nm-field__hint">{t('settings.countryHint')}</p>
      </div>

      <div className="nm-field">
        <label htmlFor="nm-interval">
          {t('settings.interval', { seconds: settings.baselineIntervalSecs })}
        </label>
        <input
          id="nm-interval"
          type="range"
          min={minIntervalSecs}
          max={maxIntervalSecs}
          step={1}
          value={settings.baselineIntervalSecs}
          onChange={(event) => {
            change({ baselineIntervalSecs: Number(event.target.value) });
          }}
        />
        <p className="nm-field__hint">{t('settings.intervalHint')}</p>
      </div>

      <div className="nm-field nm-field--inline">
        <input
          id="nm-autostart"
          type="checkbox"
          checked={settings.autostart}
          onChange={(event) => {
            change({ autostart: event.target.checked });
          }}
        />
        <label htmlFor="nm-autostart">{t('settings.autostart')}</label>
        <p className="nm-field__hint">{t('settings.autostartHint')}</p>
      </div>

      <div className="nm-field nm-field--inline">
        <input
          id="nm-remember-servers"
          type="checkbox"
          checked={settings.rememberGameServers}
          onChange={(event) => {
            change({ rememberGameServers: event.target.checked });
          }}
        />
        <label htmlFor="nm-remember-servers">{t('settings.rememberGameServers')}</label>
        <p className="nm-field__hint">{t('settings.rememberGameServersHint')}</p>
      </div>

      {/* The one setting here that buys memory rather than behaviour, so the hint states the
          figure. The snapshot date sits with it rather than on every endpoint: it describes
          the directory, not any one row, and it is the answer to "why is this named after a
          company I have never heard of" — a block that changed hands after the photograph
          was taken. It is omitted rather than guessed at while the core is still answering. */}
      <div className="nm-field nm-field--inline">
        <input
          id="nm-name-networks"
          type="checkbox"
          checked={settings.nameNetworks}
          onChange={(event) => {
            change({ nameNetworks: event.target.checked });
          }}
        />
        <label htmlFor="nm-name-networks">{t('settings.nameNetworks')}</label>
        <p className="nm-field__hint">
          {t('settings.nameNetworksHint')}{' '}
          {t('settings.nameNetworksSnapshot', { date: networkSnapshot })}
        </p>
      </div>

      <p className="nm-settings__privacy">{t('settings.privacyNote')}</p>

      {/* Quitting lives here, at the bottom of the page a user goes to on purpose.
          It used to sit in the header beside Minimize — two adjacent buttons, one of which
          hides the window and the other of which ends the monitoring, told apart by the
          muted colour of the second. Settings is reachable from every page, so the guarantee
          that a user is never stuck survives the move. */}
      <div className="nm-settings__quit">
        <p className="nm-field__hint">{t('settings.quitHint')}</p>
        <button
          type="button"
          className="nm-button"
          onClick={() => {
            void quitApp();
          }}
        >
          {t('app.quit')}
        </button>
      </div>
    </section>
  );
};
