// i18n bootstrap.
//
// Scope: this is the foundation (Phase 8 scaffold). Only a handful of
// top-level strings (tray menu, a few buttons) are extracted so far. Every
// other screen still uses English literals; they will migrate over time.
//
// Locales live next to this file under ./locales/<lang>.json. The `en.json`
// file is the source of truth — keys defined there are what every other
// locale must provide.

import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';

import en from './locales/en.json';

// Languages we ship out of the box. Each must have a JSON file in ./locales.
// When adding a language: create the JSON file, import it here, add the entry
// to `resources`, and list it in SUPPORTED_LOCALES.
export const SUPPORTED_LOCALES = [
  { code: 'en', name: 'English' },
  // Additional languages (DE/FR/ES/PT-BR/JA) will be added once the en.json
  // baseline is complete — adding them earlier just means fallback churn.
] as const;

export type LocaleCode = (typeof SUPPORTED_LOCALES)[number]['code'];

const resources: Record<string, { translation: Record<string, string> }> = {
  en: { translation: en },
};

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    fallbackLng: 'en',
    supportedLngs: SUPPORTED_LOCALES.map(l => l.code),
    interpolation: { escapeValue: false },
    detection: {
      // localStorage lets the in-app picker (once wired) override browser
      // detection; `navigator` falls through on first launch.
      order: ['localStorage', 'navigator'],
      lookupLocalStorage: 'nanocrew_locale',
      caches: ['localStorage'],
    },
  });

export default i18n;
