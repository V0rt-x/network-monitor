import { useMemo, useRef, useState } from 'react';
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
  // Whether the reader asked to change what is being watched. The picker is also open
  // whenever nothing is: then it is the only thing on the page there is to do.
  const [changing, setChanging] = useState(false);
  const search = useRef<HTMLInputElement>(null);

  const endpoints = state.kind === 'measuring' ? state.endpoints : null;
  const apps = useMemo(() => endpoints?.apps ?? [], [endpoints]);
  const watching = useMemo(() => apps.map((app) => app.name), [apps]);
  // Waiting for the core is not "nothing is being watched": showing the first-run screen
  // for the first second of every launch would tell a returning user their applications
  // were gone.
  const nothingWatched = endpoints !== null && apps.length === 0;

  // Every process of every monitored application, so the picker can mark one the user
  // never clicked — an application is a set of processes, and the ones it adopted are as
  // taken as the one that seeded it.
  const monitored = useMemo(() => {
    const owners = new Map<number, MonitoredBy>();
    for (const app of apps) {
      // Identifiers, never rendered: they are the only thing the picker's grouping and
      // the monitor's have in common, since the monitor also adopts descendants.
      for (const pid of app.pids) {
        owners.set(pid, { app: app.id, name: app.name });
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

      {/* The first run used to be an expanded picker and one grey sentence — no heading, no
          statement of what is about to happen, no primary action. For an audience installing
          this because a game stutters, the first screen has to answer "what do I press". */}
      {nothingWatched && (
        <section className="nm-apps__empty">
          <h2 className="nm-apps__emptyheading">{t('apps.empty.heading')}</h2>
          <p className="nm-apps__emptybody">{t('apps.empty.body')}</p>
          <button
            type="button"
            className="nm-button"
            onClick={() => {
              setChanging(true);
              search.current?.focus();
            }}
          >
            {t('apps.empty.action')}
          </button>
          {/* Nothing here is idle while nothing is chosen, and saying so is what stops the
              empty page reading as an application that is not working. */}
          <p className="nm-apps__emptymeanwhile">{t('apps.empty.meanwhile')}</p>
          {/* That an underline means a label explains itself is said here and in the
              help's introduction — twice in the whole product, rather than once beside each
              of two hundred figures, which is what the mark it replaces amounted to. */}
          <p className="nm-apps__emptymeanwhile">{t('help.affordance')}</p>
        </section>
      )}

      <ApplicationPicker
        monitored={monitored}
        watching={watching}
        limit={MAX_APPS}
        open={changing || nothingWatched}
        onOpenChange={setChanging}
        searchRef={search}
        onMonitor={(seedPid) => {
          void monitorApp(seedPid);
        }}
        onForget={(app) => {
          void forgetApp(app);
        }}
      />

      {state.kind === 'waiting' && <p className="nm-state--pending">{t('apps.waiting')}</p>}

      {endpoints !== null && apps.length > 0 && (
        <>
          <p className="nm-apps__window">{t('apps.window', { seconds: endpoints.windowSecs })}</p>
          <div className="nm-apps__list">
            {apps.map((app) => (
              <AppCard
                key={app.id}
                app={app}
                trafficWindowSecs={endpoints.trafficWindowSecs}
                chartStepSecs={endpoints.chartStepSecs}
                flowStatus={endpoints.flowStatus}
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
