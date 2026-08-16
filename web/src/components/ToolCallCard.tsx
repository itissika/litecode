import { useEffect, useMemo, useState } from "react";
import { FileArrowUpIcon } from "@phosphor-icons/react";

import {
  functionCallOutputText,
  normalizeToolFilePath,
  parseFunctionArguments,
} from "../api/adapter";
import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import { formatElapsed, matchJob } from "../lib/bashLive";
import { bashKill } from "../lib/litecodeBash";
import { useBashStore } from "../stores/bashStore";
import { useMessageStore } from "../stores/messageStore";
import { useTurnStore } from "../stores/turnStore";
import { agentColor } from "./agentIdentity";
import { FoldCard } from "./FoldCard";
import { ToolContentView } from "./ToolContentView";
import { ToolIcon } from "./ToolIcon";
import { deriveToolStatus } from "./toolCallStatus";

interface ToolCallCardProps {
  call: FunctionCallItem;
  output?: FunctionCallOutputItem;
  streaming?: boolean;
  projectRoot?: string | null;
  onOpenFile: (path: string) => void;
  /** Owning session id — needed by per-tool views (e.g. subagent) to resolve nested state. */
  sessionId?: string;
  /** Stable FoldCard id for virtual-list remount persistence. */
  foldCardId?: string;
}

const FILE_TOOLS = new Set(["read", "write", "edit"]);
const CMD_TOOLS = new Set(["bash", "shell", "command"]);

/** One-line collapsed summary of the tool input. */
function summarizeInput(toolName: string, input: unknown): string {
  if (input === undefined) return "";
  if (typeof input === "string") {
    return input.length > 90 ? input.slice(0, 90) + "…" : input;
  }
  if (input && typeof input === "object" && !Array.isArray(input)) {
    const obj = input as Record<string, unknown>;
    if (FILE_TOOLS.has(toolName) && typeof obj.file_path === "string") {
      return obj.file_path;
    }
    if (CMD_TOOLS.has(toolName) && typeof obj.command === "string") {
      // Prefer the human-readable `description` in the fold-card row; fall back
      // to the bare command when the tool didn't supply a description.
      const c =
        typeof obj.description === "string" ? obj.description : obj.command;
      return c.length > 90 ? c.slice(0, 90) + "…" : c;
    }
    const firstKey = Object.keys(obj)[0];
    if (firstKey) {
      const v = JSON.stringify(obj[firstKey]);
      const oneLine = `${firstKey}=${v}`;
      return oneLine.length > 90 ? oneLine.slice(0, 90) + "…" : oneLine;
    }
  }
  const asJson = JSON.stringify(input);
  return asJson.length > 90 ? asJson.slice(0, 90) + "…" : asJson;
}

function actionPayloadPath(payload: unknown): string | null {
  if (typeof payload === "string") return payload;
  return null;
}

export function ToolCallCard({
  call,
  output,
  streaming = false,
  projectRoot = null,
  onOpenFile,
  sessionId,
  foldCardId,
}: ToolCallCardProps) {
  const toolName = call.name;
  const input = parseFunctionArguments(call.arguments);
  const status = deriveToolStatus(output, streaming, call.status);
  const inputSummary = summarizeInput(toolName, input);

  // subagent_launch gets a flatter header: `subagent_launch {agent}` plus a
  // run-state dot, instead of the generic `name + inputSummary`. Its body
  // (Task brief + process list) is rendered by SubagentToolView.
  const isSubagent = toolName === "subagent_launch";
  const isBash = toolName === "bash";
  const rawOutput = output ? functionCallOutputText(output) : "";
  const bashJob = useBashStore((s) => {
    if (!isBash || !sessionId) return undefined;
    const jobs = s.bySession.get(sessionId)?.jobs ?? [];
    return matchJob(jobs, call.call_id, rawOutput);
  });
  const runningCount = useBashStore((s) =>
    sessionId ? (s.bySession.get(sessionId)?.jobs.length ?? 0) : 0,
  );
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!bashJob) return;
    const t = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(t);
  }, [bashJob]);
  const subagentAgent =
    isSubagent && input && typeof input === "object" && !Array.isArray(input)
      ? typeof (input as Record<string, unknown>).agent === "string"
        ? ((input as Record<string, unknown>).agent as string)
        : undefined
      : undefined;
  const subagentChildId = useMessageStore((s) =>
    isSubagent && call.call_id && sessionId
      ? s.bySession.get(sessionId)?.subagentBindings?.[call.call_id]
      : undefined,
  );
  const subagentRunState = useTurnStore((s) =>
    subagentChildId ? s.byId.get(subagentChildId)?.runState ?? "idle" : "idle",
  );
  const subagentRunning =
    subagentRunState === "running" || subagentRunState === "cancelling";
  const subagentColor = subagentAgent ? agentColor(subagentAgent) : undefined;

  // Action buttons are driven by tool type (not a universal copy). Today only
  // file-editing tools expose an "Open file" action; more can be added per
  // tool as needed.
  const actions = useMemo(() => {
    const acts: { id: string; label: string; kind: string; payload: unknown }[] =
      [];
    if (
      FILE_TOOLS.has(toolName) &&
      input &&
      typeof input === "object" &&
      !Array.isArray(input)
    ) {
      const filePath = (input as Record<string, unknown>).file_path;
      if (typeof filePath === "string") {
        acts.push({
          id: "open-file",
          label: "Open file",
          kind: "open_file",
          payload: filePath,
        });
      }
    }
    return acts;
  }, [toolName, input]);

  function handleAction(action: { kind: string; payload: unknown }) {
    if (action.kind === "open_file" && onOpenFile) {
      const rawPath = actionPayloadPath(action.payload);
      const path = rawPath ? normalizeToolFilePath(rawPath, projectRoot) : null;
      if (path !== null) onOpenFile(path);
    }
  }

  return (
    <FoldCard
      id={foldCardId}
      icon={<ToolIcon name={toolName} status={status} />}
      label={
        isSubagent ? (
          <span className="flex min-w-0 flex-1 items-center gap-2">
            <span className="shrink-0 font-mono text-dk-xs font-medium text-(--_dk-text-primary)">
              {toolName}
            </span>
            {subagentAgent && (
              <span
                className="truncate font-mono text-dk-2xs"
                style={subagentColor ? { color: subagentColor } : undefined}
              >
                {subagentAgent}
              </span>
            )}
            <span
              className={`ml-auto inline-block h-1.5 w-1.5 rounded-full ${
                subagentRunning || !subagentChildId
                  ? "animate-pulse bg-(--_dk-amber-500)"
                  : "bg-(--_dk-emerald-500)"
              }`}
              title={
                !subagentChildId
                  ? "Launching"
                  : subagentRunning
                    ? "Running"
                    : "Idle"
              }
            />
          </span>
        ) : (
          <span className="flex min-w-0 flex-1 items-center gap-2">
            <span className="shrink-0 font-mono text-dk-xs font-medium text-(--_dk-text-primary)">
              {toolName}
            </span>
            {inputSummary && (
              <span className="min-w-0 flex-1 truncate text-(--_dk-text-muted)">
                {inputSummary}
              </span>
            )}
            {actions.length > 0 && (
              <span className="ml-auto flex shrink-0 items-center gap-1">
                {actions.map((action) => (
                  <button
                    key={action.id}
                    type="button"
                    onClick={(e) => {
                      // The FoldCard header is a single clickable region that
                      // toggles the card; stop the click here so the action
                      // (e.g. Open file) doesn't bubble up and collapse/expand
                      // the card instead of firing.
                      e.stopPropagation();
                      handleAction(action);
                    }}
                    className="inline-flex items-center gap-1 text-dk-xs text-(--_dk-text-muted) hover:brightness-110 active:brightness-90"
                  >
                    <FileArrowUpIcon size={12} aria-hidden />
                    {action.label}
                  </button>
                ))}
              </span>
            )}
            {bashJob && (
              <span className="ml-auto flex shrink-0 items-center gap-2">
                <span className="font-mono text-dk-2xs text-(--_dk-text-muted)">
                  {formatElapsed(now - bashJob.started_at_ms)}
                </span>
                <span className="font-mono text-dk-2xs text-(--_dk-text-muted)">
                  running {runningCount}
                </span>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    void bashKill(bashJob.id);
                  }}
                  className="inline-flex items-center text-dk-xs text-(--_dk-text-muted) hover:brightness-110 active:brightness-90"
                >
                  Kill
                </button>
              </span>
            )}
          </span>
        )
      }
      streaming={streaming}
    >
      <ToolContentView
        name={toolName}
        status={status}
        input={input}
        output={output}
        callId={call.call_id}
        sessionId={sessionId}
      />
    </FoldCard>
  );
}