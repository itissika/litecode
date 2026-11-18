import { functionCallOutputText, parseFunctionArguments } from "../api/adapter";
import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import { useTurnStore } from "../stores/turnStore";
import { SubagentLaunchToolView } from "./toolviews/SubagentLaunchToolView";
import { SubagentListToolView } from "./toolviews/SubagentListToolView";
import { SubagentSendToolView } from "./toolviews/SubagentSendToolView";
import { SubagentStopToolView } from "./toolviews/SubagentStopToolView";
import { SubagentWaitToolView } from "./toolviews/SubagentWaitToolView";
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
 * Single-line auxiliary tool row (subagent_* / todo / plan / background bash).
 * No FoldCard — state and text fit on one line beside the status icon.
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
      {toolName === "subagent_wait" ? (
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
      ) : null}
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
