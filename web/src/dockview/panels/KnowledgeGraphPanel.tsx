import { useEffect, useRef, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { ReactFlowProvider } from "@xyflow/react";

import { KnowledgeDocument } from "../../components/knowledge/KnowledgeDocument";
import {
  KnowledgeModeSwitch,
  type KnowledgeSurface,
} from "../../components/knowledge/KnowledgeModeSwitch";
import { KnowledgeGraph } from "../../components/knowledge/KnowledgeGraph";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

/** The node id a restored layout stored on this panel, if any. */
function storedNodeId(params: Record<string, unknown>): string | null {
  const value = params.nodeId;
  return typeof value === "string" && value.length > 0 ? value : null;
}

export function KnowledgeGraphPanel({ api, params }: IDockviewPanelProps) {
  const [visible, setVisible] = useState(() => api.isVisible);
  const [mode, setMode] = useState<KnowledgeSurface>("document");
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const nodesLoaded = useKnowledgeStore((s) => s.nodes.length > 0);

  /*
   * Dockview persists panel params with the layout, so the document read
   * before a restart is the one this panel opens on. One-shot: an id already
   * focused in this session wins, and a stale id keeps the first-node fallback.
   */
  const restoredId = useRef<string | null>(
    useKnowledgeStore.getState().focusedId == null ? storedNodeId(params) : null,
  );
  const lastPersistedId = useRef<string | null>(storedNodeId(params));

  useEffect(() => {
    const id = restoredId.current;
    if (id == null || !nodesLoaded) return;
    restoredId.current = null;
    const state = useKnowledgeStore.getState();
    if (state.byId.has(id)) state.focusCanvas(id);
  }, [nodesLoaded]);

  // Keep the layout's copy in step, including clearing it when focus is lost.
  useEffect(() => {
    if (!nodesLoaded || restoredId.current != null) return;
    const current = useKnowledgeStore.getState().focusedId;
    if (lastPersistedId.current === current) return;
    lastPersistedId.current = current;
    api.updateParameters?.({ nodeId: current ?? undefined });
  }, [api, focusedId, nodesLoaded]);

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
