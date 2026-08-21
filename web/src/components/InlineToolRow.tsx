import { parseFunctionArguments } from "../api/adapter";
import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import { KillShellToolView } from "./toolviews/KillShellToolView";
import { WaitShellToolView } from "./toolviews/WaitShellToolView";
import { deriveToolStatus } from "./toolCallStatus";
import { ToolIcon } from "./ToolIcon";

interface InlineToolRowProps {
  call: FunctionCallItem;
  output?: FunctionCallOutputItem;
  streaming?: boolean;
  sessionId?: string;
}

/**
 * Single-line auxiliary tool row (wait_shell / kill_shell). No FoldCard — state
 * and text fit on one line beside the status icon.
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
