import { functionCallOutputText } from "../../api/adapter";
import { CodeBlock } from "../CodeBlock";
import { FoldCard } from "../FoldCard";
import type { ToolViewProps } from "./registry";

const RAW_PRE =
  "whitespace-pre-wrap break-words font-mono text-dk-sm leading-relaxed text-(--_dk-text-secondary)";

function lineCount(text: string): number {
  if (text.length === 0) return 0;
  const lines = text.split("\n");
  return lines[lines.length - 1] === "" ? lines.length - 1 : lines.length;
}

function lineLabel(count: number): string {
  return count + (count === 1 ? " line" : " lines");
}

function PanelLabel({ title, summary }: { title: string; summary: string }) {
  return (
    <span className="flex min-w-0 flex-1 items-center gap-2">
      <span className="shrink-0 font-mono font-medium text-(--_dk-text-primary)">
        {title}
      </span>
      <span className="min-w-0 truncate text-(--_dk-text-muted)">
        {summary}
      </span>
    </span>
  );
}

/**
 * Expanded body of run_code: the program as a code block, plus the raw tool
 * output. Both panels are independent FoldCards and start collapsed.
 */
export function RunCodeToolView({ input, output, status }: ToolViewProps) {
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

  const outputText = output ? functionCallOutputText(output) : "";
  const outputClass =
    status === "failed" ? RAW_PRE + " text-(--_dk-red-500)" : RAW_PRE;

  return (
    <div className="flex flex-col gap-1" data-testid="run-code-view">
      <FoldCard
        label={<PanelLabel title="code" summary={lineLabel(lineCount(code))} />}
        defaultOpen={false}
      >
        <CodeBlock code={code} lang="javascript" />
      </FoldCard>
      {outputText ? (
        <FoldCard
          label={
            <PanelLabel
              title="output"
              summary={lineLabel(lineCount(outputText))}
            />
          }
          defaultOpen={false}
        >
          <pre data-testid="run-code-output" className={outputClass}>
            {outputText}
          </pre>
        </FoldCard>
      ) : null}
    </div>
  );
}
