import React from 'react';
import ReactDOM from 'react-dom/client';
import './styles/global.css';
// Initialize i18next before any component renders. Side-effect import is
// enough — i18n/index.ts calls i18n.init() at module load.
import './i18n/index.js';
import { App } from './App.js';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
