/**
 * Map browser language tags to supported app locales.
 * Rules (WP-217): zh* → zh-CN; en* → en; everything else → en.
 */
export type AppLocale = "en" | "zh-CN";

export function resolveLocaleFromTags(tags: readonly string[]): AppLocale {
  for (const raw of tags) {
    if (!raw) continue;
    const tag = raw.trim().toLowerCase();
    if (!tag) continue;
    if (tag === "zh" || tag.startsWith("zh-")) return "zh-CN";
    if (tag === "en" || tag.startsWith("en-")) return "en";
  }
  return "en";
}

/** Read navigator.languages (with language fallback) and resolve. */
export function resolveBrowserLocale(
  languages: readonly string[] | undefined = undefined,
): AppLocale {
  if (languages !== undefined) {
    return resolveLocaleFromTags(languages);
  }
  if (typeof navigator === "undefined") {
    return "en";
  }
  const tags =
    navigator.languages && navigator.languages.length > 0
      ? Array.from(navigator.languages)
      : navigator.language
        ? [navigator.language]
        : [];
  return resolveLocaleFromTags(tags);
}
