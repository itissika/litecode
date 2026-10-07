import type { IDockviewPanelProps } from "dockview-react";

import { FileTreePanel } from "../panels/FileTreePanel";
import { SearchPanel } from "../panels/SearchPanel";
import { GitPanelHost as GitPanel } from "../panels/GitPanel";
import { EditorPanel } from "../panels/EditorPanel";
import { AgentPanel } from "../panels/AgentPanel";
import { SubagentReadOnlyPanel } from "../panels/SubagentReadOnlyPanel";
import { AboutPanel } from "../panels/AboutPanel";
import { SessionListPanel } from "../panels/SessionListPanel";
import { TerminalPanel } from "../panels/TerminalPanel";
import { KnowledgePanel } from "../panels/KnowledgePanel";
import { KnowledgeGraphPanel } from "../panels/KnowledgeGraphPanel";
import { BrowserPanel } from "../panels/BrowserPanel";

import { EdgeTab } from "../tabs/EdgeTab";
import { EditorTab } from "../tabs/EditorTab";
import { AgentTab } from "../tabs/AgentTab";
import { KnowledgeGraphTab } from "../tabs/KnowledgeGraphTab";
import { BrowserTab } from "../tabs/BrowserTab";

export const panelComponents: Record<
  string,
  React.FunctionComponent<IDockviewPanelProps>
> = {
  filetree: FileTreePanel,
  search: SearchPanel,
  git: GitPanel,
  editor: EditorPanel,
  agent: AgentPanel,
  subagent: SubagentReadOnlyPanel,
  about: AboutPanel, // registered; not in the default layout, kept for a future standalone panel
  sessions: SessionListPanel, // persistent Sessions panel on the right
  terminal: TerminalPanel,
  knowledge: KnowledgePanel,
  knowledgeGraph: KnowledgeGraphPanel,
  browser: BrowserPanel,
};

export const tabComponents: Record<
  string,
  React.FunctionComponent<IDockviewPanelProps>
> = {
  edge: EdgeTab,
  editor: EditorTab,
  agent: AgentTab,
  knowledgeGraph: KnowledgeGraphTab,
  browser: BrowserTab,
};
