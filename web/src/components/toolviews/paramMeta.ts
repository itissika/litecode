/**
 * Tool parameter metadata — the single source of truth for which fields of a
 * covered tool are rendered inline ("primary") vs. tucked into the infoicon.
 *
 * Only tools listed here get a dedicated view; everything else falls back to the
 * default JSON dump in `ToolContentView`. Adding a tool = one entry here + one
 * registration in `registry.tsx` — no changes to the dispatcher.
 */
export interface ToolParamConfig {
  /** Fields rendered inline by the view. Every other field lands in the infoicon. */
  primary: string[];
}

export const TOOL_PARAM_META: Record<string, ToolParamConfig> = {
  read: { primary: ["file_path"] },
  write: { primary: ["file_path", "content"] },
  edit: { primary: ["file_path", "edits", "old_string", "new_string"] },
  bash: { primary: ["command"] },
  wait_shell: { primary: ["id", "sec"] },
  kill_shell: { primary: ["bash_id"] },
  subagent_launch: { primary: ["agent", "prompt"] },
};

export interface MetaField {
  key: string;
  value: string;
}

/** Stringify a single parameter value for the infoicon list. */
function stringifyValue(value: unknown): string {
  if (typeof value === "string") return value;
  return JSON.stringify(value);
}

/**
 * Collect the non-primary fields of a parsed argument object into a flat
 * `{key, value}[]` for the infoicon. Returns [] for non-object inputs, empty
 * objects, or when every field is primary. Skips `undefined`/`null` values.
 */
export function collectMetaFields(
  input: unknown,
  primary: string[],
): MetaField[] {
  if (!input || typeof input !== "object" || Array.isArray(input)) return [];
  const obj = input as Record<string, unknown>;
  const primarySet = new Set(primary);
  const fields: MetaField[] = [];
  for (const key of Object.keys(obj)) {
    if (primarySet.has(key)) continue;
    const raw = obj[key];
    if (raw === undefined || raw === null) continue;
    fields.push({ key, value: stringifyValue(raw) });
  }
  return fields;
}
