import {
  Chats,
  FolderSimple,
  File,
  Robot,
  MagnifyingGlass,
  GitBranch,
} from "@phosphor-icons/react";
import type React from "react";

type IconComponent = React.FC<{ size?: number; weight?: "thin" | "light" | "regular" | "bold" }>;

/** Maps panel component name → phosphor icon. Fallback: File. */
const ICON_MAP: Record<string, IconComponent> = {
  filetree: FolderSimple,
  search: MagnifyingGlass,
  git: GitBranch,
  editor: File,
  agent: Robot,
  sessions: Chats, // session list / history
};

export function getPanelIcon(component: string): IconComponent {
  return ICON_MAP[component] ?? File;
}
