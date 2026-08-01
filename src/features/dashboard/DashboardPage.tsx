import { useTranslation } from 'react-i18next';

import { VerdictBanner } from '../../shared/VerdictBanner';
import { CoreStatusPanel } from './CoreStatusPanel';
import { GroupCard } from './GroupCard';
import { useNetworkHealth } from './useNetworkHealth';

/**
 * General network health: the domestic baseline beside the foreign one.
 *
 * Side by side because the comparison *is* the diagnosis. Domestic degraded and foreign
 * degraded together points at the user's own connection; domestic clean with foreign dead
 * points at the way out of the country. Neither column means much on its own, which is why
 * both are always rendered — even before anything has been measured, when both honestly
 * read "not measured yet".
 */
export const DashboardPage = () => {
  const { t } = useTranslation();
  const health = useNetworkHealth();

  return (
    <div className="nm-dashboard">
      {health.kind === 'waiting' && <p className="nm-state--pending">{t('dashboard.waiting')}</p>}

      {health.kind === 'unavailable' && (
        <p className="nm-state--degraded" role="alert">
          {t('dashboard.unavailable')}
        </p>
      )}

      {health.kind === 'measuring' && (
        <>
          {/* The comparison, above the two columns it is drawn from: neither baseline means
              much on its own, and the whole reason they sit side by side is what their
              disagreement says. */}
          <VerdictBanner diagnosis={health.health.diagnosis} />
          <p className="nm-dashboard__window">
            {t('dashboard.window', { seconds: health.health.windowSecs })}
          </p>
          <div className="nm-dashboard__groups">
            {health.health.groups.map((group) => (
              <GroupCard key={group.group} group={group} />
            ))}
          </div>
        </>
      )}

      <CoreStatusPanel />
    </div>
  );
};
