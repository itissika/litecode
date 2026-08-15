import type { IDockviewPanelProps } from "dockview-react";

import { FileTreePanel } from "../panels/FileTreePanel";
import { SearchPanel } from "../panels/SearchPanel";
import { EditorPanel } from "../panels/EditorPanel";
import { AgentPanel } from "../panels/AgentPanel";
import { AboutPanel } from "../panels/AboutPanel";
import { SessionListPanel } from "../panels/SessionListPanel";
import { TerminalPanel } from "../panels/TerminalPanel";

import { EdgeTab } from "../tabs/EdgeTab";
import { EditorTab } from "../tabs/EditorTab";
import { AgentTab } from "../tabs/AgentTab";

export const panelComponents: Record<string, React.FunctionComponent<IDockviewPanelProps>> = {
  filetree: FileTreePanel,
  search: SearchPanel,
  editor: EditorPanel,
  agent: AgentPanel,
  about: AboutPanel, // 已在 registry 注册但当前未在默认布局中使用，保留供未来作为独立面板打开
  sessions: SessionListPanel, // 右侧常驻 Sessions 面板
  terminal: TerminalPanel,
};

export const tabComponents: Record<string, React.FunctionComponent<IDockviewPanelProps>> = {
  edge: EdgeTab,
  editor: EditorTab,
  agent: AgentTab,
};
