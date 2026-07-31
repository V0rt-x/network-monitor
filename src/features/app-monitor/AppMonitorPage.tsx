import { useTranslation } from 'react-i18next';

import { forgetApp, monitorApp } from '../../shared/ipc';
import { AppCard } from './AppCard';
import { flowStatusKey } from './labels';
import { ProcessPicker } from './ProcessPicker';
import { useAppEndpoints } from './useAppEndpoints';

/**
 * How many applications may be monitored at once.
 *
 * Rust enforces it; this is only what the picker tells the user, and the two are stated
 * together in `CLAUDE.md` rather than one being derived from the other.
 */
const MAX_APPS = 5;

/**
 * Per-application monitoring: pick the processes, watch what each one is talking to.
 *
 * The page's whole shape follows one rule — an application is a *distribution* of endpoint
 * states, never one colour. It also has to be honest about what it cannot see: on a machine
 * without the one-time tracing setup there are no UDP endpoints and no byte counters at
 * all, and a game whose traffic is entirely UDP would otherwise appear to have no
 * connections rather than to be unobservable.
 */
export const AppMonitorPage = () => {
  const { t } = useTranslation();
  const state = useAppEndpoints();

  const endpoints = state.kind === 'measuring' ? state.endpoints : null;
  const apps = endpoints?.apps ?? [];
  const monitored = apps.map((app) => app.pid);

  return (
    <div className="nm-apps">
      {state.kind === 'unavailable' && (
        <p className="nm-state--degraded" role="alert">
          {t('apps.unavailable')}
        </p>
      )}

      {endpoints !== null && endpoints.flowStatus !== 'active' && (
        <p className="nm-apps__flow" role="status">
          {t(flowStatusKey(endpoints.flowStatus))}
        </p>
      )}

      <ProcessPicker
        monitored={monitored}
        limit={MAX_APPS}
        onMonitor={(pid) => {
          void monitorApp(pid);
        }}
        onForget={(pid) => {
          void forgetApp(pid);
        }}
      />

      {state.kind === 'waiting' && <p className="nm-state--pending">{t('apps.waiting')}</p>}

      {endpoints !== null && apps.length === 0 && (
        <p className="nm-state--pending">{t('apps.noneChosen')}</p>
      )}

      {endpoints !== null && apps.length > 0 && (
        <>
          <p className="nm-apps__window">{t('apps.window', { seconds: endpoints.windowSecs })}</p>
          <div className="nm-apps__list">
            {apps.map((app) => (
              <AppCard
                key={app.pid}
                app={app}
                trafficWindowSecs={endpoints.trafficWindowSecs}
                onForget={(pid) => {
                  void forgetApp(pid);
                }}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
};
