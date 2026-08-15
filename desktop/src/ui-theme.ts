import fs from "node:fs";
import path from "node:path";

import { app } from "electron";

export type UiThemeName = "default" | "light";

function themeFile(): string {
  return path.join(app.getPath("userData"), "ui-theme.json");
}

/** Persist UI theme for hub + workbench (shared across data: and http origins). */
export function readUiTheme(): UiThemeName {
  try {
    const raw = JSON.parse(fs.readFileSync(themeFile(), "utf8")) as { theme?: string };
    return raw.theme === "light" ? "light" : "default";
  } catch {
    return "default";
  }
}

export function writeUiTheme(theme: UiThemeName): void {
  const next: UiThemeName = theme === "light" ? "light" : "default";
  fs.mkdirSync(path.dirname(themeFile()), { recursive: true });
  const temporary = `${themeFile()}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify({ theme: next }, null, 2)}\n`, "utf8");
  fs.renameSync(temporary, themeFile());
}

/** `data-dv-theme` value consumed by tokens.css. */
export function dvThemeAttr(theme: UiThemeName): "dark" | "light" {
  return theme === "light" ? "light" : "dark";
}
