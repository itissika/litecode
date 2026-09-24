import { useEffect, useState } from "react";

import { functionCallOutputText, parseFunctionArguments } from "../api/adapter";
import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import {
  formatElapsed,
  headExitCode,
  isBashJobLive,
  matchJob,
} from "../lib/bashLive";
import { bashKill } from "../lib/litecodeBash";
import { useBashStore } from "../stores/bashStore";
import { useTurnStore } from "../stores/turnStore";
import { KillShellToolView } from "./toolviews/KillShellToolView";
import { SubagentLaunchToolView } from "./toolviews/SubagentLaunchToolView";
import { SubagentListToolView } from "./toolviews/SubagentListToolView";
import { SubagentSendToolView } from "./toolviews/SubagentSendToolView";
import { SubagentStopToolView } from "./toolviews/SubagentStopToolView";
import { SubagentWaitToolView } from "./toolviews/SubagentWaitToolView";
import { WaitShellToolView } from "./toolviews/WaitShellToolView";
import { deriveToolStatus } from "./toolCallStatus";
import { toolTitle } from "./toolviews/toolTitle";
import { ToolIcon } from "./ToolIcon";

interface InlineToolRowProps {
  call: FunctionCallItem;
  output?: FunctionCallOutputItem;
  streaming?: boolean;
  sessionId?: string;
}

/**
 * Single-line auxiliary tool row (wait_shell / kill_shell / subagent_* /
 * todo / plan / background bash). No FoldCard — state and text
 * fit on one line beside the status icon.
 */
export function InlineToolRow({
  call,
  output,
  streaming = false,
  sessionId,
}: InlineToolRowProps) {
  const toolName = call.name;
  const input = parseFunctionArguments(call.arguments);
  const status = deriveToolStatus(output, streaming, call.status);

  return (
    <div className="flex items-center gap-1.5 py-1 pl-(--_dk-indent-card-head) text-xs text-(--_dk-text-muted)">
      <ToolIcon name={toolName} status={status} streaming={streaming} />
      {toolName === "wait_shell" ? (
        <WaitShellToolView
          name={toolName}
          status={status}
          input={input}
          output={output}
          call_id={call.call_id}
          sessionId={sessionId}
        />
      ) : toolName === "subagent_wait" ? (
        <SubagentWaitToolView
          name={toolName}
          status={status}
          input={input}
          output={output}
          call_id={call.call_id}
          sessionId={sessionId}
        />
      ) : toolName === "subagent_stop" ? (
        <SubagentStopToolView
          name={toolName}
          status={status}
          input={input}
          output={output}
          call_id={call.call_id}
          sessionId={sessionId}
        />
      ) : toolName === "subagent_launch" ? (
        <SubagentLaunchToolView
          name={toolName}
          status={status}
          input={input}
          output={output}
          call_id={call.call_id}
          sessionId={sessionId}
        />
      ) : toolName === "subagent_send" ? (
        <SubagentSendToolView
          name={toolName}
          status={status}
          input={input}
          output={output}
          call_id={call.call_id}
          sessionId={sessionId}
        />
      ) : toolName === "subagent_list" ? (
        <SubagentListToolView
          name={toolName}
          status={status}
          input={input}
          output={output}
          call_id={call.call_id}
          sessionId={sessionId}
        />
      ) : toolName === "todo" || toolName === "plan" ? (
        <SummaryLine
          toolName={toolName}
          input={input}
          output={output}
          sessionId={sessionId}
        />
      ) : toolName === "bash" ? (
        <InlineBashLine
          callId={call.call_id}
          input={input}
          output={output}
          sessionId={sessionId}
        />
      ) : (
        <KillShellToolView
          name={toolName}
          status={status}
          input={input}
          output={output}
          call_id={call.call_id}
          sessionId={sessionId}
        />
      )}
    </div>
  );
}

/**
 * `todo` / `plan` row: icon + the same header summary the FoldCard uses, so the
 * session-mount capsule and the transcript agree on the wording. (The wire name
 * for the todo tool is `todo` — see `src/tool/registry.rs`.)
 */
function SummaryLine({
  toolName,
  input,
  output,
  sessionId,
}: {
  toolName: string;
  input: unknown;
  output?: FunctionCallOutputItem;
  sessionId?: string;
}) {
  const activePlanPath = useTurnStore((s) =>
    sessionId ? (s.byId.get(sessionId)?.activePlanPath ?? null) : null,
  );
  const summary = toolTitle(
    toolName,
    input,
    output ? functionCallOutputText(output) : "",
    { activePlanPath },
  ).summary;
  if (!summary) return null;
  return (
    <span
      data-testid={`inline-${toolName}-summary`}
      className="min-w-0 flex-1 truncate text-(--_dk-text-muted)"
    >
      {summary}
    </span>
  );
}

/**
 * Background `bash` row: command, live elapsed time while the job runs (with the
 * Kill action), settling to the exit code / `exited` once it is gone.
 *
 * Liveness comes from the job snapshot here — the sealed result is a one-way
 * `status: running` seal, so a vanished job means the process ended.
 */
function InlineBashLine({
  callId,
  input,
  output,
  sessionId,
}: {
  callId: string;
  input: unknown;
  output?: FunctionCallOutputItem;
  sessionId?: string;
}) {
  const rawOutput = output ? functionCallOutputText(output) : "";
  const job = useBashStore((s) => {
    if (!sessionId) return undefined;
    return matchJob(s.bySession.get(sessionId)?.jobs ?? [], callId, rawOutput);
  });
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!job) return;
    const t = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(t);
  }, [job]);

  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const command = typeof obj.command === "string" ? obj.command : "";
  const exitCode = headExitCode(rawOutput);
  const live = isBashJobLive(rawOutput, job);
  const statusText =
    live && job
      ? formatElapsed(now - job.started_at_ms)
      : exitCode !== null
        ? `exit_code: ${exitCode}`
        : "exited";

  return (
    <>
      <span
        data-testid="inline-bash-command"
        className="min-w-0 flex-1 truncate font-mono text-(--_dk-text-body)"
      >
        {command}
      </span>
      <span
        data-testid="inline-bash-status"
        className="shrink-0 font-mono text-dk-2xs text-(--_dk-text-muted)"
      >
        {statusText}
      </span>
      {live && job && (
        <button
          type="button"
          data-testid="inline-bash-kill"
          onClick={() => void bashKill(job.id)}
          className="btn-danger btn-xs shrink-0"
        >
          Kill
        </button>
      )}
    </>
  );
}
