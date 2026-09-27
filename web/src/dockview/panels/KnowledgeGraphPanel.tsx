import type { IDockviewPanelProps } from "dockview-react";
import { ReactFlowProvider } from "@xyflow/react";

import { KnowledgeGraph } from "../../components/knowledge/KnowledgeGraph";

export function KnowledgeGraphPanel(_props: IDockviewPanelProps) {
  return (
    <div className="knowledge-graph h-full min-h-0 w-full">
      <ReactFlowProvider>
        <KnowledgeGraph />
      </ReactFlowProvider>
    </div>
  );
}
