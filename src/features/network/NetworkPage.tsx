import { useTranslation } from 'react-i18next';

import { StateToken } from '../../shared/StateToken';
import { VerdictBanner } from '../../shared/VerdictBanner';
import { useFigures } from '../../shared/useFigures';
import { Distribution } from '../app-monitor/Distribution';
import { CoreStatusPanel } from '../dashboard/CoreStatusPanel';
import { MetricHelp } from '../help/MetricHelp';
import { CHECK_MARKS, checkMarkKey, checkModifier } from '../status-page/labels';
import { sectionKey } from './labels';
import { NetworkRow } from './NetworkRow';
import { useNetwork } from './useNetwork';

/**
 * One page, one list, for the one question: *is it me, my country's border, or that service?*
 *
 * It was two halves — the baselines and the service cards — and the previous phase left them
 * as two compositions on purpose. Read on a running build, that was wrong: we measured the
 * same thing and drew it twice, and the duplication went deeper than the drawing. Two of the
 * four foreign baseline targets, `discord.com` and `api.steampowered.com`, were *the same
 * addresses* as two service endpoints — half of one baseline was a second probe of a row
 * already on the page, in another visual language, under another name, spending the probe
 * budget twice for one fact.
 *
 * **What a baseline actually is: a tag, not a list.** "Domestic" and "foreign" are roles a
 * target plays — which is exactly why two of them turned out to be copies — so there is one
 * inventory, one row component, one history component and one legend. The two sections a
 * verdict is drawn from say so, which is what keeps the banner at the top checkable against
 * the rows below it.
 *
 * **The measurement layer did not merge.** `nm_core::health`'s window and `nm_core::status`'s
 * reaction rule answer different questions, and a figure computed across both would be
 * exactly the smoothing the previous phase forbade. Which rule judges a section is a property
 * of the section; each states its own cadence at level two.
 *
 * **Five names for one round trip became two.** `Ping, median` on a section heading and
 * `Ping (RTT)` on a row. *Last check* versus *mean* is a which-window qualifier and lives one
 * level down, where the distinction is made and where both still explain themselves.
 *
 * The order is the argument's order: the verdict, then what a cell means, then the evidence.
 * What the core itself is doing goes last — a fact about the app rather than about the
 * network, and the only thing on this page a reader never needs during a match.
 */
export const NetworkPage = () => {
  const { t } = useTranslation();
  const figures = useFigures();
  const state = useNetwork();

  if (state.kind === 'waiting') {
    return <p className="nm-state--pending">{t('dashboard.waiting')}</p>;
  }
  if (state.kind === 'unavailable') {
    return (
      <p className="nm-state--degraded" role="alert">
        {t('dashboard.unavailable')}
      </p>
    );
  }

  const { snapshot } = state;

  return (
    <div className="nm-network">
      <VerdictBanner diagnosis={snapshot.diagnosis} />

      {/* One legend for the whole page, where there were two histories with two
          vocabularies. It carries the three facts a reader needs about every strip — one
          cell is one check, oldest on the left, and what a cell can say — and it is where
          colour stops being the only channel, since each state is named beside its colour. */}
      <section className="nm-status__legend">
        <h2 className="nm-status__legendtitle">
          <MetricHelp topic="checks">{t('status.timeline.heading')}</MetricHelp>
        </h2>
        <ul className="nm-status__marks" aria-label={t('status.timeline.legendLabel')}>
          {CHECK_MARKS.map((mark) => (
            <li key={mark} className="nm-status__mark">
              <span className={`nm-check ${checkModifier(mark)}`} aria-hidden="true" />
              {t(checkMarkKey(mark))}
            </li>
          ))}
        </ul>
        <p className="nm-status__legendnote">{t('status.caveat')}</p>
      </section>

      {snapshot.sections.map((section) => (
        <section key={section.section} className="nm-section">
          <header className="nm-section__head">
            <h2 className="nm-section__title">{t(sectionKey(section.section))}</h2>
            {/* The marker that keeps the banner above checkable against the rows below:
                these are the sections the conclusion was drawn from, and the other two are
                not. Rust decides it, because it is the same judgement the diagnosis engine
                makes. */}
            {section.readByVerdict && (
              <span className="nm-section__evidence">
                <MetricHelp topic="verdictEvidence">
                  {t('network.section.readByVerdict')}
                </MetricHelp>
              </span>
            )}
            <StateToken health={section.verdict} />
            <span className="nm-section__rtt">
              <span className="nm-section__rttlabel">
                <MetricHelp topic="medianRtt">{t('status.metric.median')}</MetricHelp>
              </span>{' '}
              {figures.ms(section.rttMs)}
            </span>
          </header>

          <Distribution
            counts={section.counts}
            label={t('network.distribution', { row: t(sectionKey(section.section)) })}
          />

          {/* Level two, per section rather than per page: the cadence difference between a
              baseline and a platform is a number on a target now, and each section says its
              own rather than one claim being made for the whole list. */}
          <details className="nm-section__cadence">
            <summary>{t('network.cadence.summary')}</summary>
            <p>
              {t('network.cadence.detail', {
                seconds: section.cadenceSecs,
                window: section.windowSecs,
                checks: snapshot.timelinePoints,
              })}
            </p>
          </details>

          {section.rows.length === 0 ? (
            <p className="nm-state--pending">{t('status.noServices')}</p>
          ) : (
            <div className="nm-section__rows">
              {section.rows.map((row) => (
                <NetworkRow key={row.key} row={row} cadenceSecs={section.cadenceSecs} />
              ))}
            </div>
          )}
        </section>
      ))}

      <CoreStatusPanel />
    </div>
  );
};
