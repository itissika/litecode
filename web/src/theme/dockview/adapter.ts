import type { DockviewTheme } from "dockview";

/** CSS class name applied to the dockview root element. */
export const DOCKVIEW_CLASS = "litecode-dv-base";

/** Dockview theme configuration (colorScheme follows the app light/dark toggle). */
export const LITECODE_THEME: DockviewTheme = {
  name: "litecode",
  className: DOCKVIEW_CLASS,
  colorScheme: "dark",
  tabGroupIndicator: "none",
};

/** Build a dockview theme whose `colorScheme` matches the app theme. */
export function dockviewThemeForApp(appTheme: "default" | "light"): DockviewTheme {
  return {
    ...LITECODE_THEME,
    colorScheme: appTheme === "light" ? "light" : "dark",
  };
}
