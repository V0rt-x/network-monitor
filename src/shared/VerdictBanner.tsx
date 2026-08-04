import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { MetricHelp } from '../features/help/MetricHelp';
import { formatCount } from './format';
import type { DiagnosisView } from './ipc';
import { verdictAdviceKey, verdictKey, verdictModifier } from './verdict';

interface VerdictBannerProps {
  readonly diagnosis: DiagnosisView;
  /** What the verdict is about, when it is about one application. */
  readonly subject?: string;
  /**
   * "What this is drawn from", folded inside the banner's own expander.
   *
   * Only the Network page passes this: `Domestic` and `Foreign` are not services and are not
   * the user's to remove, so the evidence they add up to stays one level down from the claim
   * it supports rather than a second inventory beside it. Left undefined on every other
   * banner — an application's verdict has no baselines of its own to show.
   */
  readonly evidence?: ReactNode;
}

/**
 * The one place in the app that states a conclusion rather than a measurement.
 *
 * Three rules it exists to keep:
 *
 * **It is always shown, including when it has nothing to say.** A banner that appeared only
 * on bad news would make its absence mean "fine", and the state before anything has been
 * measured would then read as good news — which is the failure this product is built around
 * avoiding.
 *
 * **It says how much it covers.** A verdict about two of an application's seven endpoints is
 * a different message from "your game is unreachable", and partial failure inside one
 * application is the normal case under filtering rather than an edge one.
 *
 * **The advice is separate from the finding.** What was observed and what to try about it
 * are different claims: the app can see that the evidence points past the border, and it
 * cannot see whether a VPN would help.
 *
 * **The evidence is one click away, never a second inventory beside the claim.** The Network
 * page's own baselines pass their state, distribution and members as `evidence`; every other
 * caller of this component has none to show and the expander is simply absent.
 */
export const VerdictBanner = ({ diagnosis, subject, evidence }: VerdictBannerProps) => {
  const { t, i18n } = useTranslation();
  const advice = verdictAdviceKey(diagnosis.verdict);

  const scope =
    diagnosis.endpointsTotal > 0 && diagnosis.endpointsAffected > 0
      ? t('verdict.scope', {
          affected: formatCount(diagnosis.endpointsAffected, i18n.language),
          total: formatCount(diagnosis.endpointsTotal, i18n.language),
        })
      : null;

  return (
    <section
      className={`nm-verdict ${verdictModifier(diagnosis.verdict)}`}
      // Only a finding interrupts. "Not measured yet" arriving in a screen reader would be
      // noise on every start-up.
      role={diagnosis.actionable ? 'status' : undefined}
      aria-label={t('verdict.label')}
    >
      {/* The one place in the application that states a conclusion had no explanation of any
          kind — which, under the rule that a figure is not done until it has a help topic, was
          a debt rather than an omission. The headline carries it, once per banner. */}
      <p className="nm-verdict__headline">
        <MetricHelp topic="verdict">
          {subject === undefined
            ? t(verdictKey(diagnosis.verdict))
            : t('verdict.about', { subject, verdict: t(verdictKey(diagnosis.verdict)) })}
        </MetricHelp>
      </p>
      {/* How much of the application the claim is about, which is a different message from
          the claim itself and has its own topic for exactly that reason. */}
      {scope !== null && (
        <p className="nm-verdict__scope">
          <MetricHelp topic="verdictEvidence">{scope}</MetricHelp>
        </p>
      )}
      {advice !== null && <p className="nm-verdict__advice">{t(advice)}</p>}
      {evidence !== undefined && (
        <details className="nm-verdict__evidence">
          <summary>{t('verdict.evidenceSummary')}</summary>
          {evidence}
        </details>
      )}
    </section>
  );
};
