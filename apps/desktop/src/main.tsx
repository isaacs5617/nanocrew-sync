import React from 'react';
import ReactDOM from 'react-dom/client';
import * as Sentry from '@sentry/react';
import './styles/global.css';
// Initialize i18next before any component renders. Side-effect import is
// enough — i18n/index.ts calls i18n.init() at module load.
import './i18n/index.js';
import { App } from './App.js';
import { AppErrorFallback } from './components/AppErrorFallback.js';

Sentry.init({
  dsn: import.meta.env.VITE_SENTRY_DSN ?? '',
  release: import.meta.env.VITE_APP_VERSION,
  integrations: [],
  tracesSampleRate: 0,
  beforeSend(event) {
    if (event.exception) {
      event.exception.values?.forEach(ex => {
        ex.stacktrace?.frames?.forEach(frame => {
          if (frame.filename) {
            frame.filename = frame.filename.replace(/C:\\Users\\[^\\]+/gi, 'C:\\Users\\[user]');
          }
        });
      });
    }
    return event;
  },
});

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Sentry.ErrorBoundary fallback={<AppErrorFallback />}>
      <App />
    </Sentry.ErrorBoundary>
  </React.StrictMode>,
);
