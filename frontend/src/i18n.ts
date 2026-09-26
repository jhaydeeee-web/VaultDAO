import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import LanguageDetector from 'i18next-browser-languagedetector';
import HttpBackend from 'i18next-http-backend';

export const SUPPORTED_LANGUAGES = ['en', 'es', 'fr', 'ar', 'zh', 'zh-TW'];

/**
 * Map a detected language tag to a supported one. Region-specific locales we
 * ship (e.g. zh-TW) are kept as-is; anything else falls back to its base
 * language (en-US -> en).
 */
export function normalizeDetectedLanguage(lng: string): string {
  if (SUPPORTED_LANGUAGES.includes(lng)) return lng;
  return lng.split('-')[0];
}

// Configure i18next to lazily load translations from /locales/{{lng}}/{{ns}}.json
i18n
  .use(HttpBackend)
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    fallbackLng: 'en',
    supportedLngs: SUPPORTED_LANGUAGES,
    ns: ['translation'],
    defaultNS: 'translation',
    debug: false,
    backend: {
      // files served from public/locales/{lng}/{ns}.json
      loadPath: '/locales/{{lng}}/{{ns}}.json',
    },
    interpolation: {
      escapeValue: false,
    },
    react: {
      useSuspense: true,
    },
    detection: {
      order: ['localStorage', 'navigator'],
      caches: ['localStorage'],
      lookupLocalStorage: 'i18nextLng',
      convertDetectedLanguage: normalizeDetectedLanguage,
    },
  });

export default i18n;
