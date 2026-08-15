import { functionCallOutputText } from "../../api/adapter";
import type { ToolViewProps } from "./registry";

interface ParsedBashOutput {
  stdout: string;
  stderr: string;
  exitCode: string | null;
}

/**
 * Backend serializes bash output as a single string:
 *   <stdout>\nstderr:\n<stderr>\nexit_code: N
 * Split it back apart for per-stream coloring. The stderr block and exit_code
 * line are optional.
 */
function parseBashOutput(raw: string): ParsedBashOutput {
  let body = raw;
  let exitCode: string | null = null;

  const exitMatch = /\nexit_code:\s*(-?\d+)\s*$/.exec(body);
  if (exitMatch) {
    exitCode = exitMatch[1];
    body = body.slice(0, exitMatch.index);
  }

  const marker = "\nstderr:\n";
  const idx = body.indexOf(marker);
  if (idx >= 0) {
    return {
      stdout: body.slice(0, idx),
      stderr: body.slice(idx + marker.length),
      exitCode,
    };
  }
  return { stdout: body, stderr: "", exitCode };
}

/**
 * Bash tool body: the fold-card header already shows the human `description`
 * (falling back to the bare command, see `summarizeInput`), so the body here
 * just renders the actual command as a code block plus the split stdout /
 * stderr / exit_code output in a single scrollable console. Failed calls tint
 * the whole output red.
 */
export function BashToolView({ status, input, output }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const command = typeof obj.command === "string" ? obj.command : undefined;

  const rawOutput = output ? functionCallOutputText(output) : "";
  const parsed = rawOutput ? parseBashOutput(rawOutput) : null;
  const failed = status === "failed";

  return (
    <div className="flex flex-col gap-2">
      {command !== undefined && (
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md border border-(--_dk-line-visible) bg-(--_dk-surface-header) px-2 py-1.5 font-mono text-dk-sm leading-relaxed text-(--_dk-text-body)">
          {command}
        </pre>
      )}

      {parsed && (parsed.stdout || parsed.stderr || parsed.exitCode !== null) && (
        <div className="max-h-60 overflow-auto rounded-md border border-(--_dk-line-visible) font-mono text-dk-sm leading-relaxed">
          {parsed.stdout && (
            <pre
              className={`whitespace-pre-wrap break-words px-2 py-1.5 ${
                failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-secondary)"
              }`}
            >
              {parsed.stdout}
            </pre>
          )}
          {parsed.stderr && (
            <pre className="whitespace-pre-wrap break-words border-t border-(--_dk-line-visible) bg-(--_dk-amber-500)/5 px-2 py-1.5 text-(--_dk-amber-500)">
              {parsed.stderr}
            </pre>
          )}
          {parsed.exitCode !== null && (
            <span
              className={`block border-t border-(--_dk-line-visible) px-2 py-1 text-dk-xs ${
                parsed.exitCode === "0"
                  ? "text-(--_dk-text-muted)"
                  : "text-(--_dk-amber-500)"
              }`}
            >
              exit_code: {parsed.exitCode}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
