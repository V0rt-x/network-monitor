import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import './i18n';
import './styles.css';
import { App } from './App';

const container = document.getElementById('root');
if (!container) throw new Error('missing #root container');

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
