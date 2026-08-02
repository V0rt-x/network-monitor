import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { formatCount } from '../../shared/format';
import { applicationProblemKey } from './labels';
import { useApplicationList } from './useApplicationList';

/** One application already being monitored, as the picker needs to recognise it. */
export interface MonitoredBy {
  /** Identity to stop it by. */
  readonly app: number;
  /** What that application is called, so an offer can say which one already took it. */
  readonly name: string;
}

interface ApplicationPickerProps {
  /**
   * Which monitored application each running process belongs to.
   *
   * Keyed by process, because that is the only thing an offer and a monitored application
   * have in common: the offer is a grouping the picker made, the monitored one is a
   * grouping the monitor made, and they meet at the processes.
   */
  readonly monitored: ReadonlyMap<number, MonitoredBy>;
  /** How many applications are being monitored. */
  readonly count: number;
  /** How many applications may be monitored at once. */
  readonly limit: number;
  readonly onMonitor: (seedPid: number) => void;
  readonly onForget: (app: number) => void;
}

/** How many matches are rendered at once. */
const SHOWN = 40;

/**
 * The applications running on this machine, searchable, with the monitored ones marked.
 *
 * **Applications, not processes.** A raw process list is six identical `Discord.exe` rows
 * and eighty `svchost.exe` ones, and asking which of them to watch gets the question
 * backwards — the user wants Discord, and which of its processes opened the socket is
 * exactly what this product exists to stop them having to know. Rust does the grouping, by
 * the same rules the monitor uses.
 *
 * Unfiltered by network activity on purpose: a game the user wants to watch *before* it
 * connects is exactly the case where the first endpoints are worth catching.
 *
 * The list is fetched when this mounts and when the user asks again — never on a timer. It
 * is stale the instant it is taken, so polling would spend budget to be no fresher, and
 * Rust re-checks the process when monitoring actually starts.
 */
export const ApplicationPicker = ({
  monitored,
  count,
  limit,
  onMonitor,
  onForget,
}: ApplicationPickerProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;
  const { state, refresh } = useApplicationList();
  const [search, setSearch] = useState('');

  // Memoized rather than read inline so the filter below does not re-run on every render
  // of an unchanged list — this component re-renders on every keystroke.
  const applications = useMemo(
    () => (state.kind === 'listed' ? state.list.applications : []),
    [state],
  );
  const matches = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const found =
      needle === ''
        ? applications
        : applications.filter((application) => application.label.toLowerCase().includes(needle));
    return found.slice(0, SHOWN);
  }, [applications, search]);

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
          {t(applicationProblemKey(state.list.problem))}
        </p>
      )}

      {state.kind === 'listed' && state.list.problem === null && matches.length === 0 && (
        <p className="nm-state--pending">{t('apps.picker.noMatches')}</p>
      )}

      <ul className="nm-picker__list">
        {matches.map((application) => {
          // Any of its processes being monitored means the application is. The picker's
          // grouping and the monitor's need not agree exactly — the monitor also adopts
          // descendants — and the honest answer to "can I still choose this" is no as soon
          // as they overlap at all.
          const owner = application.pids
            .map((pid) => monitored.get(pid))
            .find((found) => found !== undefined);
          return (
            <li key={application.key} className="nm-picker__entry">
              <span className="nm-picker__name">{application.label}</span>
              {/* The file name beside the proper noun, whenever the bundled list supplied
                  one. A name is a claim about which program this is, and a user who cannot
                  see what it was matched against cannot tell a right name on the wrong
                  program from a right one. */}
              {application.label !== application.executable && (
                <span className="nm-picker__executable">{application.executable}</span>
              )}
              <span className="nm-picker__pid">
                {application.pids.length === 1
                  ? t('apps.pid', { pid: application.seedPid })
                  : t('apps.picker.processes', { count: application.pids.length })}
              </span>
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
                  if (owner === undefined) onMonitor(application.seedPid);
                  else onForget(owner.app);
                }}
              >
                {owner === undefined ? t('apps.start') : t('apps.stop')}
              </button>
            </li>
          );
        })}
      </ul>

      {applications.length > matches.length && (
        <p className="nm-picker__more">
          {t('apps.picker.more', {
            shown: formatCount(matches.length, locale),
            total: formatCount(applications.length, locale),
          })}
        </p>
      )}
    </section>
  );
};
