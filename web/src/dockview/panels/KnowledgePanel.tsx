import { useEffect } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { KnowledgeBrowser } from "../../components/knowledge/KnowledgeBrowser";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

export function KnowledgePanel({ api }: IDockviewPanelProps) {
  useEffect(() => {
    const note = (visible: boolean) => {
      useKnowledgeStore.getState().notePanelVisible(api.id, visible);
    };
    note(api.isVisible);
    const sub = api.onDidVisibilityChange((event) => {
      note(event.isVisible);
    });
    return () => {
      sub.dispose();
      note(false);
    };
  }, [api]);

  return (
    <div className="flex h-full min-h-0 flex-col bg-(--_dk-sidepanel)">
      <KnowledgeBrowser />
    </div>
  );
}
