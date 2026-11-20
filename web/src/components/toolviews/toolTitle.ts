export interface ToolTitle {
  summary: string;
}

const MAX_TITLE_CHARS = 72;

function truncate(value: string, max = MAX_TITLE_CHARS): string {
  const oneLine = value.replace(/\s+/g, " ").trim();
  return oneLine.length > max ? `${oneLine.slice(0, max - 1)}…` : oneLine;
}

function fallbackSummary(input: unknown): string {
  if (input == null) return "";
  if (typeof input === "string") return truncate(input);
  if (typeof input !== "object" || Array.isArray(input)) {
    return truncate(JSON.stringify(input));
  }
  const obj = input as Record<string, unknown>;
  for (const value of Object.values(obj)) {
    if (
      typeof value === "string" ||
      typeof value === "number" ||
      typeof value === "boolean"
    ) {
      return truncate(String(value));
    }
  }
  return truncate(JSON.stringify(input));
}

/** Human-oriented FoldCard header summary; dedicated per-tool titles are retired. */
export function toolTitle(
  toolName: string,
  input: unknown,
  _outputText?: string,
): ToolTitle {
  if (toolName === "run_code") return { summary: "" };
  return { summary: fallbackSummary(input) };
}
