import { useEffect } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { KnowledgeBrowser } from "../../components/knowledge/KnowledgeBrowser";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

export function KnowledgePanel({ api }: IDockviewPanelProps) {
  useEffect(() => {
    const sub = api.onDidActiveChange((event) => {
      if (event.isActive) void useKnowledgeStore.getState().refreshFromDisk();
    });
    return () => sub.dispose();
  }, [api]);

  return (
    <div className="flex h-full min-h-0 flex-col bg-(--_dk-sidepanel)">
      <KnowledgeBrowser />
    </div>
  );
}
