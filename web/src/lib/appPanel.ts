import type { AppView } from "../api/apps";
import { dockview } from "../dockview/workbench/host";
import { closePanel, openPanel } from "../dockview/workbench/commands";
import i18n from "../i18n";

export const APP_VIEW_PREFIX = "app-";

export function appViewPanelId(appId: string): string {
  return `${APP_VIEW_PREFIX}${appId}`;
}

/** Open or focus the singleton panel for a running app. */
export function openAppPanel(app: AppView): void {
  const id = appViewPanelId(app.id);
  const live = dockview();
  const existing = live?.getPanel(id);
  if (existing) {
    existing.api.setTitle?.(app.name);
    existing.api.updateParameters?.({ appId: app.id, url: app.url });
    existing.api.setActive();
    return;
  }
  openPanel({
    id,
    component: "appview",
    title: app.name,
    tabComponent: "app",
    params: { appId: app.id, url: app.url },
  });
}

export function closeAppPanel(appId: string): void {
  closePanel(appViewPanelId(appId));
}

export const GALLERY_PANEL_ID = "gallery";

/** Bundled apps carry an English catalog name; their panel title is localized. */
export function builtinAppName(app: Pick<AppView, "builtin_view" | "name">): string {
  return app.builtin_view === "gallery" ? i18n.t("gallery.title") : app.name;
}

/**
 * Open the panel behind a bundled app. These apps have no process, so this is
 * the only way in; an unknown view is ignored rather than opening a blank tab.
 */
export function openBuiltinAppView(view: string): void {
  if (view !== "gallery") return;
  const live = dockview();
  const existing = live?.getPanel(GALLERY_PANEL_ID);
  if (existing) {
    existing.api.setTitle?.(i18n.t("gallery.title"));
    existing.api.setActive();
    return;
  }
  openPanel({
    id: GALLERY_PANEL_ID,
    component: "gallery",
    title: i18n.t("gallery.title"),
    tabComponent: "default",
  });
}

