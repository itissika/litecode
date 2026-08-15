import { functionCallOutputText } from "../api/adapter";
import type { FunctionCallOutputItem } from "../api/types";
import type { ToolStatus } from "./ToolIcon";
import { getToolView, viewOwnsOutput } from "./toolviews/registry";

const RESOURCE_BUSY_RE = /resource busy(,\s*held by session\s+(\S+))?/i;

interface ResourceBusy {
  heldBy: string | null;
}

function detectResourceBusy(text: string | undefined): ResourceBusy | null {
  if (!text) return null;
  const m = RESOURCE_BUSY_RE.exec(text);
  if (!m) return null;
  return { heldBy: m[2] ?? null };
}

function formatInput(input: unknown): string {
  if (input === undefined) return "";
  if (typeof input === "string") return input;
  return JSON.stringify(input, null, 2);
}

interface ToolContentViewProps {
  name: string;
  status: ToolStatus;
  input: unknown;
  output?: FunctionCallOutputItem;
  /** Tool call id — used by per-tool views (e.g. subagent) to resolve nested state. */
  callId?: string;
  /** Owning session id — used by per-tool views to look up their bindings. */
  sessionId?: string;
}

/**
 * Expanded body of a tool call. A thin dispatcher: detects resource-busy, then
 * delegates to a registered per-tool view when one exists, otherwise renders the
 * default input (JSON dump) + output (text). Covered views own their input;
 * output is rendered here unless the view opts to own it (`viewOwnsOutput`).
 */
export function ToolContentView({
  name,
  status,
  input,
  output,
  callId,
  sessionId,
}: ToolContentViewProps) {
  const resultContent = output ? functionCallOutputText(output) : undefined;
  const resourceBusy = detectResourceBusy(
    typeof resultContent === "string" ? resultContent : undefined,
  );

  if (resourceBusy) {
    return (
      <div className="flex items-start gap-2 rounded-md bg-(--_dk-amber-500)/10 px-2 py-1.5 text-dk-sm leading-relaxed text-(--_dk-amber-500)">
        <span>
          {resourceBusy.heldBy
            ? `Resource is held by session ${resourceBusy.heldBy.slice(0, 8)}. It will be retried automatically once released — switch sessions or wait.`
            : "Resource is held by another session. It will be retried automatically once released."}
        </span>
      </div>
    );
  }

  const ToolView = getToolView(name);
  const hasInput = input !== undefined && input !== "";
  const hasResult = typeof resultContent === "string" && resultContent.length > 0;
  const ownsOutput = ToolView !== undefined && viewOwnsOutput(name);

  return (
    <div className="flex flex-col gap-2">
      {ToolView ? (
        <ToolView
          name={name}
          status={status}
          input={input}
          output={output}
          call_id={callId}
          sessionId={sessionId}
        />
      ) : (
        hasInput && (
          <pre className="whitespace-pre-wrap break-words font-mono text-dk-sm leading-relaxed text-(--_dk-text-secondary)">
            {formatInput(input)}
          </pre>
        )
      )}

      {!ownsOutput && hasResult && (
        <pre
          className={`whitespace-pre-wrap break-words font-mono text-dk-sm leading-relaxed ${
            status === "failed"
              ? "text-(--_dk-red-500)"
              : "text-(--_dk-text-secondary)"
          }`}
        >
          {resultContent}
        </pre>
      )}
    </div>
  );
}
