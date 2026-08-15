export interface MenuItem {
  label: string;
  items: MenuEntry[];
}

export type MenuEntry = string | "separator";

export function buildMenuItems(_sessionMode: "local" | "remote"): MenuItem[] {
  // Workspace open / remote attach live on the Electron Home hub only.
  return [
    {
      label: "Options",
      items: [
        "Home",
        "separator",
        "Settings...",
        "Theme: Dark",
        "Theme: Light",
        "separator",
        "About",
      ],
    },
  ];
}

/** Default menu (local) for static imports / tests. */
export const menuItems: MenuItem[] = buildMenuItems("local");
