import {
  Chats,
  FolderSimple,
  File,
  Robot,
  MagnifyingGlass,
} from "@phosphor-icons/react";
import type React from "react";

type IconComponent = React.FC<{ size?: number; weight?: "thin" | "light" | "regular" | "bold" }>;

/** Maps panel component name → phosphor icon. Fallback: File. */
const ICON_MAP: Record<string, IconComponent> = {
  filetree: FolderSimple,
  search: MagnifyingGlass,
  editor: File,
  agent: Robot,
  sessions: Chats, // 历史对话（会话列表）
};

export function getPanelIcon(component: string): IconComponent {
  return ICON_MAP[component] ?? File;
}
