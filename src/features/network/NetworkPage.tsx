import { useTranslation } from 'react-i18next';

import { CoreStatusPanel } from '../dashboard/CoreStatusPanel';
import { DashboardPage } from '../dashboard/DashboardPage';
import { StatusPage } from '../status-page/StatusPage';

/**
 * One page for the one question: *is it me, my country's border, or that service?*
 *
 * The baselines and the service cards were two tabs, and answering that question meant
 * switching between them while holding one page in your head to read the other. They are
 * halves of a single argument: the verdict engine reads the baselines, and the service
 * cards are the evidence a reader checks that verdict against.
 *
 * **The merge is an arrangement, not a blend.** The two halves are measured on different
 * cadences by different rules — `nm_core::health`'s window over the baselines,
 * `nm_core::status`'s reaction rule over the checks — and each states its own on the page
 * rather than having the difference smoothed over. Nothing about a card's or a baseline's
 * own behaviour changes here.
 *
 * The order is the argument's order: the verdict, then the baselines it was drawn from,
 * then the services to check it against. What the core itself is doing goes last — it is a
 * fact about the app rather than about the network, and it is the only thing on this page a
 * reader never needs during a match.
 */
export const NetworkPage = () => {
  const { t } = useTranslation();

  return (
    <div className="nm-network">
      <section className="nm-network__half" aria-labelledby="nm-network-baselines">
        <h2 className="nm-network__heading" id="nm-network-baselines">
          {t('network.baselines')}
        </h2>
        <DashboardPage />
      </section>

      <section className="nm-network__half" aria-labelledby="nm-network-services">
        <h2 className="nm-network__heading" id="nm-network-services">
          {t('network.services')}
        </h2>
        <StatusPage />
      </section>

      <CoreStatusPanel />
    </div>
  );
};
