import type { ReactElement } from "react";
import {
  rowsToNodes,
  groupNodes,
  NodeView,
  ProcessGroup,
  processGroupHasTerminalStop,
} from "./MessageList";
import { processGroupAutoOpen } from "./toolCallStatus";
import { displayMessages, useMessageStore } from "../stores/messageStore";
import { useTurnStore } from "../stores/turnStore";
import { useSessionStore } from "../stores/sessionStore";
import { useEditorStore } from "../stores/editorStore";

/**
 * Lightweight, NON-virtualized transcript viewport for a child (subagent)
 * session. Reuses the exact same row primitives as the main MessageList so the
 * visual language is identical, but renders as a plain flex column that the
 * parent FoldCard scrolls — never a nested virtualizer (which would break
 * measure). A subagent that itself launches a subagent recurses through the
 * same NodeView → ToolCallCard → SubagentToolView path, so nesting is free.
 *
 * `nested` indents the process list (left border + padding) to show depth. The
 * top-level subagent passes `nested={false}` so its process aligns flush with
 * the Task brief above it; a subagent launched *inside* another subagent passes
 * `nested={true}`.
 */
export function SubagentViewport({
  childSessionId,
  nested = false,
}: {
  childSessionId: string;
  nested?: boolean;
}): ReactElement {
  const messages = useMessageStore((s) =>
    displayMessages(s.bySession.get(childSessionId)),
  );
  const runState = useTurnStore(
    (s) => s.byId.get(childSessionId)?.runState ?? "idle",
  );
  const project = useSessionStore((s) => s.project);
  const openFile = useEditorStore((s) => s.openFile);

  const isRunning = runState === "running" || runState === "cancelling";
  const nodes = rowsToNodes(messages);
  const groups = groupNodes(nodes);

  if (messages.length === 0 && !isRunning) {
    return (
      <p className="px-1 py-1 text-dk-2xs italic text-(--_dk-text-disabled)">
        Empty subagent session
      </p>
    );
  }

  return (
    <div
      className={
        nested
          ? "flex flex-col gap-1 border-l border-(--_dk-ix-bg-hover) pl-(--_dk-indent-step)"
          : "flex flex-col gap-1"
      }
    >
      {groups.map((group, gi) => {
        if (group.type === "process") {
          const hasLive = group.nodes.some((n) => n.kind !== "compact_cut" && n.live);
          const followedByMessage = groups[gi + 1]?.type === "output";
          const hasTerminalStop = processGroupHasTerminalStop(group.nodes);
          return (
            <ProcessGroup
              key={`proc-${gi}`}
              nodes={group.nodes}
              streaming={hasLive}
              autoOpen={processGroupAutoOpen({
                followedByMessage,
                hasTerminalStop,
              })}
              sessionId={childSessionId}
              groupIndex={gi}
            />
          );
        }
        return group.nodes.map((n) => (
          <NodeView
            key={n.key}
            node={n}
            streaming={n.streaming}
            sessionId={childSessionId}
            projectRoot={project}
            onOpenFile={(path) => void openFile(path)}
          />
        ));
      })}
    </div>
  );
}
