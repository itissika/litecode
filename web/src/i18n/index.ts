import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json" with { type: "json" };
import zhCN from "./locales/zh-CN.json" with { type: "json" };
import { resolveBrowserLocale } from "./resolveLocale";

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    "zh-CN": { translation: zhCN },
  },
  lng: resolveBrowserLocale(),
  fallbackLng: "en",
  supportedLngs: ["en", "zh-CN"],
  nonExplicitSupportedLngs: false,
  interpolation: { escapeValue: false },
  // Missing keys return the key string — never throw.
  returnNull: false,
  returnEmptyString: false,
  parseMissingKeyHandler: (key) => key,
});

export default i18n;
export { resolveBrowserLocale, resolveLocaleFromTags } from "./resolveLocale";
export type { AppLocale } from "./resolveLocale";
