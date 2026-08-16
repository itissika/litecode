import { useEffect, useMemo } from "react";
import type { ReactElement } from "react";
import { FileTextIcon } from "@phosphor-icons/react";

import type { ToolViewProps } from "./registry";
import { AgentMarkdown } from "../AgentMarkdown";
import { useConnectionStore } from "../../stores/connectionStore";
import { useMessageStore } from "../../stores/messageStore";
import { useTurnStore } from "../../stores/turnStore";
import { FoldCard, FOLDCARD_HEADER_TONE } from "../FoldCard";
import { SubagentViewport } from "../SubagentViewport";

/** Prompts longer than this collapse by default in the task FoldCard. */
const TASK_PREVIEW_LIMIT = 200;

interface SubagentInput {
  agent?: string;
  prompt?: string;
}

function parseSubagentInput(input: unknown): SubagentInput {
  if (!input || typeof input !== "object" || Array.isArray(input)) return {};
  const obj = input as Record<string, unknown>;
  return {
    agent: typeof obj.agent === "string" ? obj.agent : undefined,
    prompt: typeof obj.prompt === "string" ? obj.prompt : undefined,
  };
}

/**
 * Tool view for `subagent_launch`. Resolves the durable child session id from
 * the parent session's `subagentBindings` (keyed by call_id), subscribes to the
 * child session so the backend replays its transcript, and renders it as a
 * nested, collapsible viewport. The card itself stays a normal tool FoldCard
 * (header/collapse handled by ToolCallCard), so this view only owns the body.
 */
export function SubagentToolView({
  input,
  call_id,
  sessionId,
}: ToolViewProps): ReactElement {
  const childId = useMessageStore((s) =>
    call_id && sessionId
      ? s.bySession.get(sessionId)?.subagentBindings?.[call_id]
      : undefined,
  );
  const connState = useConnectionStore((s) => s.state);

  // A subagent is "nested" when the session it lives in (sessionId) is itself a
  // child session of another — i.e. sessionId appears as a bound child somewhere.
  // Nested viewports indent their process list to show depth; the top-level one
  // aligns flush with the Task brief above it.
  const isNested = useMessageStore((s) => {
    if (!sessionId) return false;
    for (const slice of s.bySession.values()) {
      if (Object.values(slice.subagentBindings).includes(sessionId)) return true;
    }
    return false;
  });

  // Subscribe to the child session for its lifetime. The backend replies to
  // `session/subscribe` with a `session/snapshot` that triggers the initial
  // `buffer/load` + turn snapshot — so no manual buffer load is needed. Cleanup
  // unsubscribes and clears the child slice to avoid orphan subscriptions / leaks.
  useEffect(() => {
    if (!childId || connState !== "connected") return;
    let cancelled = false;
    let attempts = 0;
    const trySubscribe = () => {
      if (cancelled) return;
      useConnectionStore
        .getState()
        .ensureSubscribe(childId)
        .catch((err: unknown) => {
          const msg = err instanceof Error ? err.message : "";
          if (!cancelled && /session.*not found/i.test(msg) && attempts < 5) {
            attempts += 1;
            window.setTimeout(trySubscribe, 400 * attempts);
          }
        });
    };
    trySubscribe();
    return () => {
      cancelled = true;
      useConnectionStore.getState().unsubscribeSession(childId);
      useMessageStore.getState().reset(childId);
      useTurnStore.getState().resetTurn(childId);
    };
  }, [childId, connState]);

  const { prompt } = useMemo(() => parseSubagentInput(input), [input]);
  const taskLong = !!prompt && prompt.length > TASK_PREVIEW_LIMIT;

  // The agent name + run-state dot live in the outer tool FoldCard header (see
  // ToolCallCard), so the body only renders the two flat, same-level sections:
  // the Task brief (the subagent's *input*) and the process list (its *output*).
  return (
    <div className="flex flex-col gap-1.5">
      {prompt ? (
        <FoldCard
          icon={<FileTextIcon size={12} aria-hidden />}
          label={
            <span className="flex min-w-0 flex-1 items-center gap-2">
              <span className={`${FOLDCARD_HEADER_TONE} shrink-0 font-mono text-dk-xs font-medium text-(--_dk-text-primary)`}>
                Task
              </span>
              <span className={`${FOLDCARD_HEADER_TONE} min-w-0 flex-1 truncate text-(--_dk-text-muted)`}>
                {prompt}
              </span>
            </span>
          }
          defaultOpen={!taskLong}
          streaming={false}
        >
          <div className="tool-card-markdown">
            <AgentMarkdown text={prompt} />
          </div>
        </FoldCard>
      ) : (
        <p className="px-1 py-1 text-dk-2xs italic text-(--_dk-text-muted)">
          No task description
        </p>
      )}

      {childId ? (
        <SubagentViewport childSessionId={childId} nested={isNested} />
      ) : (
        <p className="px-1 py-1 text-dk-2xs italic text-(--_dk-text-disabled)">
          Launching subagent…
        </p>
      )}
    </div>
  );
}
