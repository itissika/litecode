import type { IDockviewPanelProps } from "dockview-react";

import { LazyEditorPanel } from "./lazyEditorPanel";

import { FileTreePanel } from "../panels/FileTreePanel";
import { GalleryPanel } from "../../components/gallery/GalleryPanel";
import { SearchPanel } from "../panels/SearchPanel";
import { GitPanelHost as GitPanel } from "../panels/GitPanel";
import { AgentPanel } from "../panels/AgentPanel";
import { SubagentReadOnlyPanel } from "../panels/SubagentReadOnlyPanel";
import { AboutPanel } from "../panels/AboutPanel";
import { SessionListPanel } from "../panels/SessionListPanel";
import { TerminalPanel } from "../panels/TerminalPanel";
import { KnowledgePanel } from "../panels/KnowledgePanel";
import { KnowledgeGraphPanel } from "../panels/KnowledgeGraphPanel";
import { BrowserPanel } from "../panels/BrowserPanel";
import { AppsPanel } from "../../components/apps/AppsPanel";
import { AppViewPanel } from "../panels/AppViewPanel";

import { EdgeTab } from "../tabs/EdgeTab";
import { EditorTab } from "../tabs/EditorTab";
import { AgentTab } from "../tabs/AgentTab";
import { KnowledgeGraphTab } from "../tabs/KnowledgeGraphTab";
import { BrowserTab } from "../tabs/BrowserTab";
import { AppTab } from "../tabs/AppTab";
import { DefaultTab } from "../tabs/DefaultTab";

export const panelComponents: Record<
  string,
  React.FunctionComponent<IDockviewPanelProps>
> = {
  filetree: FileTreePanel,
  gallery: GalleryPanel,
  search: SearchPanel,
  git: GitPanel,
  editor: LazyEditorPanel,
  agent: AgentPanel,
  subagent: SubagentReadOnlyPanel,
  about: AboutPanel, // registered; not in the default layout, kept for a future standalone panel
  sessions: SessionListPanel, // persistent Sessions panel on the right
  terminal: TerminalPanel,
  knowledge: KnowledgePanel,
  knowledgeGraph: KnowledgeGraphPanel,
  browser: BrowserPanel,
  apps: AppsPanel,
  appview: AppViewPanel,
};

export const tabComponents: Record<
  string,
  React.FunctionComponent<IDockviewPanelProps>
> = {
  default: DefaultTab,
  edge: EdgeTab,
  editor: EditorTab,
  agent: AgentTab,
  knowledgeGraph: KnowledgeGraphTab,
  browser: BrowserTab,
  app: AppTab,
};
