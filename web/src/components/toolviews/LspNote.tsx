import { InfoIcon, WarningCircleIcon } from "@phosphor-icons/react";

import { functionCallOutputText } from "../../api/adapter";
import type { FunctionCallOutputItem } from "../../api/types";
import { FoldCard } from "../FoldCard";

/**
 * Write/edit results may append a Warning, Error, or Hint block after the main body
 * (see tool ACI signal grammar). Split so the main message stays inline and the
 * signal tail can collapse.
 */
const SIGNAL_RE = /\n\n(?:Warning|Error|Hint):/;

export function splitLspNote(text: string): { body: string; lsp?: string } {
  const m = SIGNAL_RE.exec(text);
  if (m && m.index !== undefined) {
    return {
      body: text.slice(0, m.index).trimEnd(),
      lsp: text.slice(m.index + 2).trim(),
    };
  }
  return { body: text, lsp: undefined };
}

/** Collapsed tail card for Warning/Error/Hint appendix on write/edit. */
export function LspNoteTail({ text }: { text: string }) {
  const trimmed = text.trim();
  const isError = /^Error:/i.test(trimmed);
  const isHint = /^Hint:/i.test(trimmed);
  return (
    <FoldCard
      icon={
        isHint ? (
          <InfoIcon
            size={12}
            aria-hidden
            className="text-(--_dk-text-muted)"
          />
        ) : (
          <WarningCircleIcon
            size={12}
            aria-hidden
            className={isError ? "text-(--_dk-red-500)" : "text-(--_dk-amber-500)"}
          />
        )
      }
      label={isError ? "Tool error note" : isHint ? "LSP note" : "Tool warning"}
      defaultOpen={false}
    >
      <pre className="whitespace-pre-wrap break-words font-mono text-dk-xs leading-relaxed text-(--_dk-text-secondary)">
        {text}
      </pre>
    </FoldCard>
  );
}

/**
 * Renders a write/edit tool result: the main message inline, plus a collapsed
 * signal tail when present. Returns null when there is no output text at all.
 */
export function ToolResultBlock({ output }: { output?: FunctionCallOutputItem }) {
  const text = output ? functionCallOutputText(output) : "";
  if (!text) return null;
  const { body, lsp } = splitLspNote(text);
  return (
    <div className="flex flex-col gap-1">
      {body && (
        <div className="text-dk-xs leading-relaxed text-(--_dk-text-muted)">
          {body}
        </div>
      )}
      {lsp && <LspNoteTail text={lsp} />}
    </div>
  );
}
