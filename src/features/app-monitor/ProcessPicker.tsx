import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { formatCount } from '../../shared/format';
import { processProblemKey } from './labels';
import { useProcessList } from './useProcessList';

/** One application a listed process already belongs to. */
export interface MonitoredBy {
  /** Identity to stop it by. */
  readonly app: number;
  /** What that application is called, so the entry can say which one took the process. */
  readonly name: string;
}

interface ProcessPickerProps {
  /**
   * Which application each already-monitored process belongs to.
   *
   * Keyed by process rather than by application because that is the question this list
   * asks of every row — and because one application holds several of them, so a process
   * the user did not pick can still turn out to be taken.
   */
  readonly monitored: ReadonlyMap<number, MonitoredBy>;
  /** How many applications are being monitored. */
  readonly count: number;
  /** How many applications may be monitored at once. */
  readonly limit: number;
  readonly onMonitor: (pid: number) => void;
  readonly onForget: (app: number) => void;
}

/** How many matches are rendered at once. */
const SHOWN = 40;

/**
 * The list of running processes, searchable, with the monitored ones marked.
 *
 * Unfiltered by network activity on purpose: a game the user wants to watch *before* it
 * connects is exactly the case where the first endpoints are worth catching, and a picker
 * that only offered processes already holding a socket would hide it.
 *
 * The list is fetched when this mounts and when the user asks again — never on a timer. A
 * process list is stale the instant it is taken, so polling would spend budget to be no
 * fresher, and Rust re-checks the identifier when monitoring actually starts.
 *
 * What a click starts is an *application*, not this process: Rust forms one around it from
 * its namesakes, its descendants and any bundled preset. So a process the user never picked
 * can appear here as already taken, and the entry says which application took it — a
 * grouping nobody can inspect is one nobody can correct.
 */
export const ProcessPicker = ({
  monitored,
  count,
  limit,
  onMonitor,
  onForget,
}: ProcessPickerProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const { state, refresh } = useProcessList();
  const [search, setSearch] = useState('');

  // Memoized rather than read inline so the filter below does not re-run on every render
  // of an unchanged list — a few hundred processes is small, but this component also
  // re-renders on every keystroke.
  const processes = useMemo(() => (state.kind === 'listed' ? state.list.processes : []), [state]);
  const matches = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const found =
      needle === ''
        ? processes
        : processes.filter((process) => process.name.toLowerCase().includes(needle));
    return found.slice(0, SHOWN);
  }, [processes, search]);

  const full = count >= limit;

  return (
    <section className="nm-picker">
      <header className="nm-picker__header">
        <h3 className="nm-picker__title">{t('apps.picker.heading')}</h3>
        <button
          type="button"
          className="nm-button nm-button--quiet"
          onClick={refresh}
          disabled={state.kind === 'loading'}
        >
          {t('apps.picker.refresh')}
        </button>
      </header>

      <p className="nm-picker__hint">{t('apps.picker.hint', { limit })}</p>

      <label className="nm-picker__search">
        <span>{t('apps.picker.searchLabel')}</span>
        <input
          type="search"
          value={search}
          placeholder={t('apps.picker.searchPlaceholder')}
          onChange={(event) => {
            setSearch(event.target.value);
          }}
        />
      </label>

      {state.kind === 'loading' && <p className="nm-state--pending">{t('apps.picker.loading')}</p>}

      {state.kind === 'unavailable' && (
        <p className="nm-state--degraded" role="alert">
          {t('apps.picker.unavailable')}
        </p>
      )}

      {state.kind === 'listed' && state.list.problem !== null && (
        <p className="nm-state--degraded" role="alert">
          {t(processProblemKey(state.list.problem))}
        </p>
      )}

      {state.kind === 'listed' && state.list.problem === null && matches.length === 0 && (
        <p className="nm-state--pending">{t('apps.picker.noMatches')}</p>
      )}

      <ul className="nm-picker__list">
        {matches.map((process) => {
          const owner = monitored.get(process.pid);
          return (
            <li key={process.pid} className="nm-picker__entry">
              <span className="nm-picker__name">{process.name}</span>
              <span className="nm-picker__pid">{t('apps.pid', { pid: process.pid })}</span>
              {owner !== undefined && (
                <span className="nm-picker__owner">
                  {t('apps.picker.partOf', { name: owner.name })}
                </span>
              )}
              <button
                type="button"
                className="nm-button"
                // The cap is enforced in Rust; disabling here explains *why* nothing would
                // happen rather than letting the click fail silently.
                disabled={owner === undefined && full}
                onClick={() => {
                  if (owner === undefined) onMonitor(process.pid);
                  else onForget(owner.app);
                }}
              >
                {owner === undefined ? t('apps.start') : t('apps.stop')}
              </button>
            </li>
          );
        })}
      </ul>

      {processes.length > matches.length && (
        <p className="nm-picker__more">
          {t('apps.picker.more', {
            shown: formatCount(matches.length, locale),
            total: formatCount(processes.length, locale),
          })}
        </p>
      )}
    </section>
  );
};
