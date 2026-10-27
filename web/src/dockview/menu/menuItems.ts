import i18n from "../../i18n";

export interface MenuItem {
  label: string;
  items: MenuEntry[];
}

/** Display entry; `id` is the stable action token passed to onAction. */
export type MenuEntry =
  | { id: string; label: string }
  | "separator";

export function buildMenuItems(_sessionMode: "local" | "remote"): MenuItem[] {
  // Workspace open / remote attach live on the Electron Home hub only.
  return [
    {
      label: i18n.t("menu.options"),
      items: [
        { id: "Home", label: i18n.t("menu.home") },
        "separator",
        { id: "Settings...", label: i18n.t("menu.settings") },
        { id: "Theme: Dark", label: i18n.t("menu.themeDark") },
        { id: "Theme: Light", label: i18n.t("menu.themeLight") },
        "separator",
        { id: "About", label: i18n.t("menu.about") },
      ],
    },
  ];
}
