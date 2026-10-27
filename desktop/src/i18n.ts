import fs from "node:fs";
import path from "node:path";

import i18n from "i18next";

import { resolveLocale, type AppLocale } from "./resolveLocale";

type LocaleJson = Record<string, unknown>;

/** Resolve a locale JSON file (dist copy first, then web source fallbacks — same idea as theme tokens). */
function loadLocaleJson(fileName: string): LocaleJson {
  const candidates = [
    path.join(__dirname, "i18n", fileName),
    path.resolve(__dirname, "../../web/src/i18n/locales", fileName),
    path.resolve(process.cwd(), "../web/src/i18n/locales", fileName),
    path.resolve(process.cwd(), "web/src/i18n/locales", fileName),
  ];
  for (const candidate of candidates) {
    try {
      return JSON.parse(fs.readFileSync(candidate, "utf8")) as LocaleJson;
    } catch {
      // try next
    }
  }
  throw new Error(
    `i18n locale not found: ${fileName} (expected desktop/dist/i18n/${fileName} or web/src/i18n/locales/${fileName})`,
  );
}

function loadResources(): { en: LocaleJson; "zh-CN": LocaleJson } {
  return {
    en: loadLocaleJson("en.json"),
    "zh-CN": loadLocaleJson("zh-CN.json"),
  };
}

let ready = false;

/**
 * Init i18next for the Electron main / hub HTML builder.
 * Resources come from the copied dist dictionaries (or web source in dev).
 */
export function initDesktopI18n(lng: AppLocale): typeof i18n {
  const resources = loadResources();
  if (!ready) {
    void i18n.init({
      resources: {
        en: { translation: resources.en },
        "zh-CN": { translation: resources["zh-CN"] },
      },
      lng,
      fallbackLng: "en",
      supportedLngs: ["en", "zh-CN"],
      nonExplicitSupportedLngs: false,
      interpolation: { escapeValue: false },
      returnNull: false,
      returnEmptyString: false,
      parseMissingKeyHandler: (key) => key,
      initAsync: false,
    });
    ready = true;
  } else {
    void i18n.changeLanguage(lng);
  }
  return i18n;
}

/**
 * Ensure i18n is ready (defaults to `en` when callers outside the hub
 * path — components / ssh progress — need `t()` before Electron locale is known).
 */
export function ensureDesktopI18n(lng: AppLocale = "en"): typeof i18n {
  return ready ? i18n : initDesktopI18n(lng);
}

/** Translate a semantic id key. Safe to call after `ensureDesktopI18n` / hub init. */
export function t(key: string, options?: Record<string, unknown>): string {
  ensureDesktopI18n();
  return String(options ? i18n.t(key, options) : i18n.t(key));
}

export { resolveLocale };
export type { AppLocale };
