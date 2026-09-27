import type { IDockviewPanelProps } from "dockview-react";

import { KnowledgeBrowser } from "../../components/knowledge/KnowledgeBrowser";

export function KnowledgePanel(_props: IDockviewPanelProps) {
  return (
    <div className="flex h-full min-h-0 flex-col bg-(--_dk-sidepanel)">
      <KnowledgeBrowser />
    </div>
  );
}
