import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import './i18n';
// uPlot's own stylesheet, bundled rather than fetched — the app makes no network request
// the user did not ask for, and that includes its own assets.
import 'uplot/dist/uPlot.min.css';
import './styles.css';
import { App } from './App';

const container = document.getElementById('root');
if (!container) throw new Error('missing #root container');

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
