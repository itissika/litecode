import { useEffect, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { ReactFlowProvider } from "@xyflow/react";

import { KnowledgeDocument } from "../../components/knowledge/KnowledgeDocument";
import {
  KnowledgeModeSwitch,
  type KnowledgeSurface,
} from "../../components/knowledge/KnowledgeModeSwitch";
import { KnowledgeGraph } from "../../components/knowledge/KnowledgeGraph";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

export function KnowledgeGraphPanel({ api }: IDockviewPanelProps) {
  const [visible, setVisible] = useState(() => api.isVisible);
  const [mode, setMode] = useState<KnowledgeSurface>("document");
  const focusedId = useKnowledgeStore((s) => s.focusedId);

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
    <div className="relative flex h-full min-h-0 w-full flex-col bg-(--_dk-editor)">
      {mode === "document" ? (
        // Keyed by node: switching documents resets the draft/editor state and
        // runs the outgoing document's save-on-unmount.
        <KnowledgeDocument key={focusedId ?? "empty"} />
      ) : (
        <div
          className="knowledge-graph min-h-0 w-full flex-1"
          style={{ display: visible ? undefined : "none" }}
          aria-hidden={!visible}
        >
          <ReactFlowProvider>
            <KnowledgeGraph />
          </ReactFlowProvider>
        </div>
      )}
      <KnowledgeModeSwitch mode={mode} onChange={setMode} />
    </div>
  );
}
