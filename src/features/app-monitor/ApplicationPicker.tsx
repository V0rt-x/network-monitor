import type { Ref } from 'react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { formatCount } from '../../shared/format';
import { MetricHelp } from '../help/MetricHelp';
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
  /** What is being watched, in the order the cards are in, for the folded line. */
  readonly watching: readonly string[];
  /** How many applications may be monitored at once. */
  readonly limit: number;
  /** Whether the list, the filter and the scope toggle are on screen. */
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** So the empty state's own action can put the cursor where the choosing happens. */
  readonly searchRef?: Ref<HTMLInputElement>;
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
 * **Named applications only, and Rust decides which those are.** There used to be a "show
 * everything running" checkbox and a line reading "89 processes hidden — nothing here knows
 * a name for them": the product explaining its own catalogue to someone who wants to click
 * their game. `list_applications` now returns only what the bundled index has a name for, so
 * the payload shrinks with the list rather than the UI filtering a list it was sent anyway.
 * *Variant C, chosen by the user on 2026-08-04 after the cost was stated* — an application
 * the index has no name for is unwatchable, and the fix is an entry in a release rather than
 * anything the user can do. Phase 7's five-title pass is where that risk gets measured.
 *
 * Unfiltered by network activity on purpose: a game the user wants to watch *before* it
 * connects is exactly the case where the first endpoints are worth catching.
 *
 * The list is fetched when this mounts and when the user asks again — never on a timer. It
 * is stale the instant it is taken, so polling would spend budget to be no fresher, and
 * Rust re-checks the process when monitoring actually starts.
 *
 * **It folds away once something is being watched.** This is a setup tool, not a surface to
 * watch — a heading, a refresh, a hint, a scope checkbox, a count, a filter and up to forty
 * scrolling rows — and it was mounted permanently above the measurements, taking the whole
 * first screen. Folded it is one line naming what is being watched. Open is the state it
 * starts in when nothing has been chosen yet, because then there is nothing else on the page
 * and choosing is the only thing to do.
 *
 * The hint about arming the monitor before a match belongs to the open state for the same
 * reason: it is worth reading once, on the first run, rather than sitting above the figures
 * for the rest of the application's life.
 */
export const ApplicationPicker = ({
  monitored,
  watching,
  limit,
  open,
  onOpenChange,
  searchRef,
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

  const full = watching.length >= limit;

  // Folded: one line saying what is being watched, and the way back in. Everything below is
  // the setup tool, and a setup tool does not belong above the measurements.
  if (!open) {
    return (
      <section className="nm-picker nm-picker--folded">
        <p className="nm-picker__watching">
          <MetricHelp topic="watching">
            {watching.length === 0
              ? t('apps.picker.watchingNone')
              : t('apps.picker.watching', {
                  names: watching.join(', '),
                  count: watching.length,
                  limit,
                })}
          </MetricHelp>
        </p>
        <button
          type="button"
          className="nm-button"
          onClick={() => {
            onOpenChange(true);
          }}
        >
          {t('apps.picker.change')}
        </button>
      </section>
    );
  }

  return (
    <section className="nm-picker">
      <header className="nm-picker__header">
        {/* What choosing an application actually does — that it takes the whole process
            group and anything it launches, and that nothing inside a process is ever read —
            was three sentences here. It is a real answer to a real question and it is not a
            warning, so it moved a level down rather than off the page. */}
        <h3 className="nm-picker__title">
          <MetricHelp topic="watching">{t('apps.picker.chooseHeading')}</MetricHelp>
        </h3>
        <div className="nm-picker__actions">
          <button
            type="button"
            className="nm-button nm-button--quiet"
            onClick={refresh}
            disabled={state.kind === 'loading'}
          >
            {t('apps.picker.refresh')}
          </button>
          <button
            type="button"
            className="nm-button"
            onClick={() => {
              onOpenChange(false);
            }}
          >
            {t('apps.picker.done')}
          </button>
        </div>
      </header>

      {/* Shown only while the picker is open, which for most sessions means once. It is the
          one thing worth knowing before choosing and the one thing a returning user does
          not need above their figures. */}
      <p className="nm-picker__hint">{t('apps.picker.chooseHint', { limit })}</p>

      <label className="nm-picker__search">
        <span>{t('apps.picker.searchLabel')}</span>
        <input
          type="search"
          ref={searchRef}
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

      {/*
       * A grid, one row per application, aligned down the list.
       *
       * Every entry was a wrapping flex line, so the action button landed under a different
       * word on every row and one line of content took four rem of height. Identity on the
       * left, one action on the right, and the count as a chip rather than as a sentence.
       */}
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
              <span className="nm-picker__identity">
                <span className="nm-picker__name">{application.label}</span>
                {/* How large a group the rule caught, which is the one thing about the
                    grouping worth a glance. Which processes they are is the product's
                    implementation showing through, and Rust no longer sends it. */}
                <span className="nm-count">
                  <span className="nm-count__value">
                    {formatCount(application.pids.length, locale)}
                  </span>{' '}
                  <span className="nm-count__state">
                    {t('apps.picker.processWord', { count: application.pids.length })}
                  </span>
                </span>
                {owner !== undefined && (
                  <span className="nm-picker__owner">
                    {t('apps.picker.partOf', { name: owner.name })}
                  </span>
                )}
              </span>
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
