import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from './locales/en.json'

/**
 * English is the source language and the only one shipped so far. The layer
 * exists from the first screen anyway: retrofitting it means touching every
 * string in the product, and a self-hosted tool lands in teams that do not
 * work in English.
 */
void i18n.use(initReactI18next).init({
  resources: { en: { translation: en } },
  lng: 'en',
  fallbackLng: 'en',
  interpolation: { escapeValue: false },
})

export default i18n
