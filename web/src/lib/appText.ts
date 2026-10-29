import type { AppView } from "../api/apps";
import i18n from "../i18n";

/**
 * The required one-line app description in the UI language. Registries may
 * ship translations; an exact tag wins, then its base language, then English,
 * then the catalog default.
 */
export function appDescription(
  app: Pick<AppView, "description" | "description_i18n">,
  locale: string = i18n.language,
): string {
  const texts = app.description_i18n ?? {};
  const base = locale.split("-")[0];
  return texts[locale] ?? texts[base] ?? texts.en ?? app.description;
}
