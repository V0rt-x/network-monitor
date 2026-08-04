import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { StateToken } from '../../shared/StateToken';
import { VerdictBanner } from '../../shared/VerdictBanner';
import { useFigures } from '../../shared/useFigures';
import { Distribution } from '../app-monitor/Distribution';
import { CoreStatusPanel } from '../dashboard/CoreStatusPanel';
import { MetricHelp } from '../help/MetricHelp';
import { sectionKey } from './labels';
import { NetworkCatalogueEditor } from './NetworkCatalogueEditor';
import { NetworkTile } from './NetworkTile';
import { useNetwork } from './useNetwork';

/**
 * One page, one list, for the one question: *is it me, my country's border, or that service?*
 *
 * **Network becomes the user's own page again.** *Phase 6.8 items 17–20, reversing and
 * refining Phase 6.7 item 27 after the user read the running build.* The previous phase
 * folded the verdict's own baselines and the user's services into four headings under one
 * banner; read on screen, `Domestic` and `Foreign` crowded out the thing a player actually
 * came to edit. They are not services and are not the user's to remove, so they moved one
 * level down — inside the banner's own "what this is drawn from" expander — and what remains
 * on the page is the user's own catalogue, in tiles, with an `Edit` control over it.
 *
 * **The measurement layer did not merge.** `nm_core::health`'s window and `nm_core::status`'s
 * reaction rule answer different questions, and a figure computed across both would be
 * exactly the smoothing the previous phase forbade. Which rule judges a section is a property
 * of the section; each states its own cadence at level two.
 *
 * **Editing changes what is shown, never what is measured for the verdict.** `Domestic` and
 * `Foreign` are probed and reported whatever the catalogue selection says, because thinning
 * the verdict's own sample by unticking a tile that happens to double as evidence — Steam and
 * Discord are both a gaming platform and foreign evidence — would be exactly the kind of
 * silent measurement change this product exists not to make. Rust enforces this; the page
 * only ever asks Rust which sections those are.
 *
 * The order is the argument's order: the verdict and its evidence, then the user's own
 * services, grouped and tiled. What the core itself is doing goes last — a fact about the app
 * rather than about the network, and the only thing on this page a reader never needs during
 * a match.
 */
export const NetworkPage = () => {
  const { t } = useTranslation();
  const figures = useFigures();
  const state = useNetwork();
  const [editing, setEditing] = useState(false);

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
  const evidenceSections = snapshot.sections.filter(
    (section) => section.readByVerdict && section.rows.length > 0,
  );
  const tileSections = snapshot.sections.filter(
    (section) => !section.readByVerdict && section.rows.length > 0,
  );

  return (
    <div className="nm-network">
      <VerdictBanner
        diagnosis={snapshot.diagnosis}
        evidence={
          evidenceSections.length === 0 ? undefined : (
            <div className="nm-verdict__evidencelist">
              {evidenceSections.map((section) => (
                <div key={section.section} className="nm-verdict__evidencesection">
                  <header className="nm-verdict__evidencehead">
                    <h3 className="nm-verdict__evidencetitle">{t(sectionKey(section.section))}</h3>
                    <StateToken health={section.verdict} />
                    <span className="nm-verdict__evidencertt">
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
                  <div className="nm-verdict__evidencerows">
                    {section.rows.map((row) => (
                      <NetworkTile key={row.key} row={row} cadenceSecs={section.cadenceSecs} />
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )
        }
      />

      <NetworkCatalogueEditor open={editing} onOpenChange={setEditing} />

      {tileSections.map((section) => (
        <section key={section.section} className="nm-section">
          <header className="nm-section__head">
            <h2 className="nm-section__title">{t(sectionKey(section.section))}</h2>
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

          <div className="nm-network__tiles">
            {section.rows.map((row) => (
              <NetworkTile key={row.key} row={row} cadenceSecs={section.cadenceSecs} />
            ))}
          </div>
        </section>
      ))}

      <CoreStatusPanel />
    </div>
  );
};
