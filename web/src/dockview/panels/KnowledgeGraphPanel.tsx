import { useEffect, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { ReactFlowProvider } from "@xyflow/react";

import { KnowledgeGraph } from "../../components/knowledge/KnowledgeGraph";

export function KnowledgeGraphPanel({ api }: IDockviewPanelProps) {
  const [visible, setVisible] = useState(() => api.isVisible);

  useEffect(() => {
    setVisible(api.isVisible);
    const sub = api.onDidVisibilityChange((event) => {
      setVisible(event.isVisible);
    });
    return () => sub.dispose();
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
