/**
 * Map Electron `app.getLocale()` (or any BCP-47 / OS tag) to supported app locales.
 * Rules (WP-217): zh* → zh-CN; en* → en; everything else → en.
 */
export type AppLocale = "en" | "zh-CN";

export function resolveLocale(locale: string): AppLocale {
  const tag = String(locale ?? "")
    .trim()
    .toLowerCase()
    .replace(/_/g, "-");
  if (!tag) return "en";
  if (tag === "zh" || tag.startsWith("zh-")) return "zh-CN";
  if (tag === "en" || tag.startsWith("en-")) return "en";
  return "en";
}
