const MAX_TITLE_CHARS = 90;
const MAX_TOKEN_CHARS = 24;

export interface ToolTitle {
  summary: string;
}

function truncate(value: string, max = MAX_TITLE_CHARS): string {
  return value.length > max ? `${value.slice(0, max)}…` : value;
}

function stringField(input: Record<string, unknown>, name: string): string | null {
  const value = input[name];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function formatScope(input: Record<string, unknown>, fields: string[]): string {
  for (const field of fields) {
    const value = stringField(input, field);
    if (value) return ` · in ${value}`;
  }
  return "";
}

function todoSummary(outputText?: string): string {
  const match = outputText?.match(
    /Status\s+—\s+pending:\s*(\d+),\s*in_progress:\s*(\d+),\s*completed:\s*(\d+)/,
  );
  if (!match) return "Updating tasks…";
  const [, pending, active, completed] = match;
  return `${active} active · ${pending} pending · ${completed} done`;
}

function planSummary(
  input: Record<string, unknown>,
  outputText?: string,
  activePlanPath?: string | null,
): string {
  const created = outputText?.match(/^Created plan at\s+(.+?)(?:\r?\n|$)/m)?.[1]?.trim();
  if (created) return created;
  if (outputText?.includes("Active plan cleared.")) return "Active plan cleared";
  if (input.action === "create" && activePlanPath) return activePlanPath;
  return input.action === "finish" ? "Clearing active plan…" : "Creating plan…";
}

function lspSummary(input: Record<string, unknown>): string {
  const action = stringField(input, "action");
  const filePath = stringField(input, "file_path");
  const label: Record<string, string> = {
    goToDefinition: "def",
    definition: "def",
    findReferences: "refs",
    references: "refs",
    hover: "hover",
    diagnostics: "diag",
  };
  const line = typeof input.line === "number" ? `:${input.line}` : "";
  const text = stringField(input, "text");
  const token = text ? ` ${truncate(text, MAX_TOKEN_CHARS)}` : "";
  return [label[action ?? ""] ?? action, filePath ? `${filePath}${line}${token}` : null]
    .filter(Boolean)
    .join(" · ");
}

function fallbackSummary(input: unknown): string {
  if (input === undefined) return "";
  if (typeof input === "string") return truncate(input);
  if (input && typeof input === "object" && !Array.isArray(input)) {
    const obj = input as Record<string, unknown>;
    const preferred = Object.entries(obj).find(
      ([, value]) => typeof value === "string" && value.trim(),
    );
    const [key, value] = preferred ?? Object.entries(obj)[0] ?? [];
    if (key) return truncate(`${key}=${JSON.stringify(value)}`);
  }
  return truncate(JSON.stringify(input));
}

/** Human-oriented FoldCard title text. Keep execution mechanics in the body. */
export function toolTitle(
  toolName: string,
  input: unknown,
  outputText?: string,
  context: { activePlanPath?: string | null } = {},
): ToolTitle {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    return { summary: fallbackSummary(input) };
  }
  const obj = input as Record<string, unknown>;

  if (toolName === "todo") return { summary: todoSummary(outputText) };
  if (toolName === "plan") {
    return { summary: planSummary(obj, outputText, context.activePlanPath) };
  }
  if (toolName === "lsp") return { summary: lspSummary(obj) };

  if (toolName === "grep") {
    const regex = stringField(obj, "regex");
    return { summary: regex ? truncate(`/${regex}/${formatScope(obj, ["path", "include_pattern"])}`) : fallbackSummary(input) };
  }
  if (toolName === "glob") {
    const pattern = stringField(obj, "pattern");
    return { summary: pattern ? truncate(`${pattern}${formatScope(obj, ["path"])}`) : fallbackSummary(input) };
  }
  if (["code_search", "session_search"].includes(toolName)) {
    const query = stringField(obj, "query");
    return {
      summary: query
        ? truncate(`${query}${formatScope(obj, ["include_pattern", "session_filter"])}`)
        : fallbackSummary(input),
    };
  }
  if (toolName === "websearch") {
    return { summary: stringField(obj, "query") ?? fallbackSummary(input) };
  }
  if (toolName === "webfetch") {
    return { summary: stringField(obj, "url") ?? fallbackSummary(input) };
  }
  if (["read", "write", "edit"].includes(toolName)) {
    return { summary: stringField(obj, "file_path") ?? fallbackSummary(input) };
  }
  if (["bash", "shell", "command"].includes(toolName)) {
    const description = stringField(obj, "description");
    const command = stringField(obj, "command");
    if (toolName === "bash") {
      // The bash schema has no `description` field (agents sometimes add it as
      // an extra arg, but usually omit it) — the command is the reliable,
      // informative title.
      return { summary: truncate(command ?? description ?? "") };
    }
    return { summary: truncate(description ?? command ?? "") };
  }
  return { summary: fallbackSummary(input) };
}
