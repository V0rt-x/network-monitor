import { useTranslation } from 'react-i18next';

import { CoreStatusPanel } from './features/dashboard/CoreStatusPanel';

export const App = () => {
  const { t } = useTranslation();

  return (
    <main className="nm-app">
      <h1 className="nm-app__title">{t('app.name')}</h1>
      <p className="nm-app__tagline">{t('app.tagline')}</p>
      <CoreStatusPanel />
    </main>
  );
};
