import { functionCallOutputText } from "../../api/adapter";
import { CodeBlock } from "../CodeBlock";
import { collectMetaFields, TOOL_PARAM_META } from "./paramMeta";
import type { ToolViewProps } from "./registry";

const RAW_PRE =
  "whitespace-pre-wrap break-words font-mono text-dk-sm leading-relaxed text-(--_dk-text-secondary)";

const RUN_CODE_DATA_MARK = "@@run_code_data\n";

interface RunCodeCall {
  seq?: number;
  tool?: string;
  outcome?: string;
  error_code?: string;
}

interface RunCodeLog {
  level?: string;
  message?: string;
}

interface RunCodeDiagnostic {
  code?: string;
  severity?: string;
  message?: string;
}

interface RunCodeData {
  calls: RunCodeCall[];
  logs: RunCodeLog[];
  diagnostics: RunCodeDiagnostic[];
}

/** Parse the structured ledger appended to a `run_code` text render. */
export function parseRunCodeLedger(text: string): RunCodeData | null {
  const at = text.lastIndexOf(RUN_CODE_DATA_MARK);
  if (at < 0) return null;
  try {
    const value = JSON.parse(text.slice(at + RUN_CODE_DATA_MARK.length).trim()) as {
      calls?: unknown;
      logs?: unknown;
      diagnostics?: unknown;
    };
    if (!value || typeof value !== "object" || !Array.isArray(value.calls)) return null;
    return {
      calls: value.calls as RunCodeCall[],
      logs: Array.isArray(value.logs) ? (value.logs as RunCodeLog[]) : [],
      diagnostics: Array.isArray(value.diagnostics)
        ? (value.diagnostics as RunCodeDiagnostic[])
        : [],
    };
  } catch {
    return null;
  }
}

/**
 * Expanded body of `run_code`: the program, plus calls, logs, and diagnostics
 * when the result carries structured data. Older transcripts keep their text.
 */
export function RunCodeToolView({ name, input, output }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : null;
  const code = obj && typeof obj.code === "string" ? obj.code : null;

  if (code === null) {
    const raw =
      typeof input === "string"
        ? input
        : input === undefined
          ? ""
          : JSON.stringify(input, null, 2);
    return raw ? <pre className={RAW_PRE}>{raw}</pre> : null;
  }

  const meta = collectMetaFields(
    input,
    TOOL_PARAM_META[name]?.primary ?? ["code"],
  );

  const outputText = output ? functionCallOutputText(output) : "";
  const ledger = outputText ? parseRunCodeLedger(outputText) : null;

  return (
    <div className="flex flex-col gap-1" data-testid="run-code-view">
      {meta.length > 0 && (
        <div className="flex flex-wrap gap-x-3 gap-y-1 font-mono text-dk-2xs text-(--_dk-text-muted)">
          {meta.map((field) => (
            <span key={field.key}>
              {field.key}: {field.value}
            </span>
          ))}
        </div>
      )}
      <CodeBlock code={code} lang="javascript" />
      {ledger ? (
        <div data-testid="run-code-ledger" className="flex flex-col gap-1">
          {ledger.calls.length > 0 && (
            <ul data-testid="run-code-calls" className={RAW_PRE}>
              {ledger.calls.map((call, index) => (
                <li key={`${call.seq ?? index}-${call.tool ?? ""}`}>
                  {call.seq ?? index + 1}. {call.tool ?? "?"} {call.outcome ?? "?"}
                  {call.error_code ? ` ${call.error_code}` : ""}
                </li>
              ))}
            </ul>
          )}
          {ledger.logs.length > 0 && (
            <ul data-testid="run-code-logs" className={RAW_PRE}>
              {ledger.logs.map((entry, index) => (
                <li key={index}>
                  {entry.level ?? "log"}: {entry.message ?? ""}
                </li>
              ))}
            </ul>
          )}
          {ledger.diagnostics.length > 0 && (
            <ul data-testid="run-code-diagnostics" className={RAW_PRE}>
              {ledger.diagnostics.map((item, index) => (
                <li key={index}>
                  {item.severity ?? "info"} {item.code ?? "?"}: {item.message ?? ""}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : (
        outputText && (
          <pre data-testid="run-code-fallback" className={RAW_PRE}>
            {outputText}
          </pre>
        )
      )}
    </div>
  );
}
