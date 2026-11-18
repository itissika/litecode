import { functionCallOutputText } from "../../api/adapter";
import { matchJobByCallId } from "../../lib/terminalLive";
import { useBashStore } from "../../stores/bashStore";
import {
  CommandHeader,
  LiveTerminalView,
  PinnedOutput,
} from "./LiveTerminalView";
import type { ToolViewProps } from "./registry";

function commandLabel(name: string, input: unknown): string | undefined {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    return undefined;
  }
  const obj = input as Record<string, unknown>;
  if (name === "exec_command") {
    return typeof obj.cmd === "string" ? obj.cmd : undefined;
  }
  if (name === "write_stdin") {
    const sid = typeof obj.session_id === "string" ? obj.session_id : "";
    const chars = typeof obj.chars === "string" ? obj.chars : "";
    if (!sid && !chars) return undefined;
    return chars ? `${sid} · ${JSON.stringify(chars)}` : `${sid} · (poll)`;
  }
  return undefined;
}

/**
 * Unified exec view (`exec_command` / `write_stdin`): renders the live tail
 * while attached to a background job, or the returned command output directly
 * without legacy text-format parsing.
 */
export function ExecToolView({
  name,
  status,
  input,
  output,
  call_id,
  sessionId,
}: ToolViewProps) {
  const command = commandLabel(name, input);
  const rawOutput = output ? functionCallOutputText(output) : "";
  const failed = status === "failed";

  const job = useBashStore((s) => {
    if (!sessionId) return undefined;
    return matchJobByCallId(s.bySession.get(sessionId)?.jobs ?? [], call_id);
  });

  if (job) {
    return (
      <LiveTerminalView
        job={job}
        command={command ?? job.command_preview}
        sessionId={sessionId}
        failed={failed}
        dataExecCallId={call_id}
      />
    );
  }

  if (command === undefined && !rawOutput) {
    return null;
  }

  return (
    <div
      className="overflow-hidden rounded-md border border-(--_dk-line-visible) bg-(--_dk-editor) font-mono text-dk-sm leading-relaxed"
      data-testid="exec-console"
      data-exec-call-id={call_id}
    >
      {command !== undefined && <CommandHeader command={command} />}
      {rawOutput && <PinnedOutput text={rawOutput} failed={failed} />}
    </div>
  );
}
