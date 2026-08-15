/** Supported theme names. */
export type ThemeName = "default" | "light";

const STORAGE_KEY = "litecode-theme";

/** Custom event fired when the theme changes. */
export const THEME_CHANGE_EVENT = "litecode-theme-change";

/** Persist and apply a theme. */
export function setTheme(name: ThemeName): void {
  try {
    localStorage.setItem(STORAGE_KEY, name);
  } catch {
    // localStorage not available — still apply
  }
  void window.litecode?.setUiTheme?.(name);
  applyTheme(name);
}

/** Apply theme to DOM without persisting (used on initial load). */
export function applyTheme(name: ThemeName): void {
  // Theme is document-global: a single data-dv-theme attribute on <html>
  // drives :root token overrides, so every node (including portals) is themed.
  syncDockviewTheme(name);

  window.dispatchEvent(new CustomEvent(THEME_CHANGE_EVENT, { detail: name }));
}

/** Read persisted theme or default. Desktop host preference wins over localStorage. */
export function getTheme(): ThemeName {
  const fromHost = window.litecode?.getUiTheme?.();
  if (fromHost === "light" || fromHost === "default") {
    return fromHost;
  }
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "light") return "light";
    if (stored === "default") return "default";
  } catch {
    // localStorage not available
  }
  return "default";
}

/** Set data-dv-theme on <html> so :root token overrides apply document-wide. */
function syncDockviewTheme(name: ThemeName) {
  const dvTheme = name === "light" ? "light" : "dark";
  document.documentElement.setAttribute("data-dv-theme", dvTheme);
}

/** Toggle between light and default themes. */
export function toggleTheme(): ThemeName {
  const next = getTheme() === "light" ? "default" : "light";
  setTheme(next);
  return next;
}
