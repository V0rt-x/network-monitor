import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import { forgetApp, monitorApp } from '../../shared/ipc';
import { AppCard } from './AppCard';
import { flowStatusKey } from './labels';
import type { MonitoredBy } from './ApplicationPicker';
import { ApplicationPicker } from './ApplicationPicker';
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
  const apps = useMemo(() => endpoints?.apps ?? [], [endpoints]);

  // Every process of every monitored application, so the picker can mark one the user
  // never clicked — an application is a set of processes, and the ones it adopted are as
  // taken as the one that seeded it.
  const monitored = useMemo(() => {
    const owners = new Map<number, MonitoredBy>();
    for (const app of apps) {
      for (const process of app.processes) {
        owners.set(process.pid, { app: app.id, name: app.name });
      }
    }
    return owners;
  }, [apps]);

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

      <ApplicationPicker
        monitored={monitored}
        count={apps.length}
        limit={MAX_APPS}
        onMonitor={(seedPid) => {
          void monitorApp(seedPid);
        }}
        onForget={(app) => {
          void forgetApp(app);
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
                key={app.id}
                app={app}
                trafficWindowSecs={endpoints.trafficWindowSecs}
                onForget={(id) => {
                  void forgetApp(id);
                }}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
};
