import { useEffect, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { ReactFlowProvider } from "@xyflow/react";

import { KnowledgeGraph } from "../../components/knowledge/KnowledgeGraph";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

export function KnowledgeGraphPanel({ api }: IDockviewPanelProps) {
  const [visible, setVisible] = useState(() => api.isVisible);

  useEffect(() => {
    const note = (visible: boolean) => {
      setVisible(visible);
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
    <div
      className="knowledge-graph h-full min-h-0 w-full"
      style={{ display: visible ? undefined : "none" }}
      aria-hidden={!visible}
    >
      <ReactFlowProvider>
        <KnowledgeGraph />
      </ReactFlowProvider>
    </div>
  );
}
