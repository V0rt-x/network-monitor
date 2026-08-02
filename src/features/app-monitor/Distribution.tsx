import { useTranslation } from 'react-i18next';

import { formatCount } from '../../shared/format';
import type { HealthCountsView } from '../../shared/ipc';
import { healthModifier } from '../dashboard/labels';

/**
 * Which counts are worth showing, and in what order.
 *
 * The same severity order Rust sorts endpoints by, so the list and the summary above it read
 * the same way round: nothing gets through, then degraded, then filtered — an absence of
 * knowledge sits below being told "no" — then alive but unmeasured, then unmeasured, then
 * clean.
 */
const ENTRIES = [
  { key: 'unreachable', labelKey: 'dashboard.health.unreachable' },
  { key: 'degraded', labelKey: 'dashboard.health.degraded' },
  { key: 'blocked', labelKey: 'dashboard.health.blocked' },
  { key: 'carryingTraffic', labelKey: 'dashboard.health.carryingTraffic' },
  { key: 'unknown', labelKey: 'dashboard.health.unknown' },
  { key: 'ok', labelKey: 'dashboard.health.ok' },
] as const;

interface DistributionProps {
  readonly counts: HealthCountsView;
  /**
   * What this distribution is a distribution *of*.
   *
   * The page shows more than one — the application as a whole, and each of its transport
   * groups — and a bare list of "1 unreachable, 2 degraded" read aloud in sequence would be
   * three tallies of nothing in particular.
   */
  readonly label: string;
  /** Extra classes for the list, so a group header can lay it out differently. */
  readonly className?: string;
}

/**
 * How many endpoints are in each state — never a single colour standing for all of them.
 *
 * "4 clean, 2 degraded, 1 unreachable" is a fact a user can act on. One badge for a whole
 * application, or for a whole group of its endpoints, is either an outage that is not
 * happening or a failure being hidden, and partial failure is the normal case under
 * filtering rather than an edge one.
 *
 * States with no members are left out rather than shown as zeros: the point is what *is*
 * happening.
 */
export const Distribution = ({ counts, label, className }: DistributionProps) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.language;

  const present = ENTRIES.map((entry) => ({ ...entry, value: counts[entry.key] })).filter(
    (entry) => entry.value > 0,
  );
  if (present.length === 0) return null;

  return (
    <ul className={`nm-distribution ${className ?? ''}`.trimEnd()} aria-label={label}>
      {present.map((entry) => (
        <li key={entry.key} className={`nm-health ${healthModifier(entry.key)}`}>
          {t('dashboard.distributionEntry', {
            amount: formatCount(entry.value, locale),
            state: t(entry.labelKey),
          })}
        </li>
      ))}
    </ul>
  );
};
