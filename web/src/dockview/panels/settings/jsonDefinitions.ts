import type {
  ArgMatcher,
  CustomToolDefinition,
  McpServerDefinition,
  PermissionActionName,
  ToolPermissionRule,
} from "../../../api/settings";
import type { SerializeResult } from "./persist";

export const TOOL_ID_RE = /^[a-z][a-z0-9_]*$/;

export function parseCustomToolJson(
  text: string,
  expectedName?: string | null,
): SerializeResult<CustomToolDefinition> {
  try {
    const raw = JSON.parse(text) as Record<string, unknown>;
    if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
      return { skip: "invalid" };
    }
    const name = typeof raw.name === "string" ? raw.name.trim() : "";
    if (!TOOL_ID_RE.test(name)) return { skip: "invalid" };
    if (expectedName && name !== expectedName) return { skip: "invalid" };
    const command = typeof raw.command === "string" ? raw.command.trim() : "";
    if (!command) return { skip: "invalid" };
    const schemaRaw = raw.schema;
    if (
      schemaRaw === null ||
      typeof schemaRaw !== "object" ||
      Array.isArray(schemaRaw)
    ) {
      return { skip: "invalid" };
    }
    const schema = schemaRaw as Record<string, unknown>;
    const properties = schema.properties ?? {};
    if (
      properties === null ||
      typeof properties !== "object" ||
      Array.isArray(properties)
    ) {
      return { skip: "invalid" };
    }
    const required = schema.required ?? [];
    if (
      !Array.isArray(required) ||
      required.some((x) => typeof x !== "string")
    ) {
      return { skip: "invalid" };
    }
    const args = Array.isArray(raw.args)
      ? raw.args.filter((x): x is string => typeof x === "string")
      : [];
    const timeout =
      typeof raw.timeout === "number" && raw.timeout > 0 ? raw.timeout : 120;
    const rules = parseRules(raw.rules);
    if (rules === "invalid") return { skip: "invalid" };
    const def: CustomToolDefinition = {
      name,
      description:
        typeof raw.description === "string" ? raw.description.trim() : "",
      command,
      args,
      timeout,
      schema: {
        type: typeof schema.type === "string" ? schema.type : "object",
        properties: properties as Record<string, unknown>,
        required: required as string[],
      },
    };
    if (rules) def.rules = rules;
    return { ok: def };
  } catch {
    return { skip: "invalid" };
  }
}

const PERMISSION_ACTIONS = new Set<PermissionActionName>([
  "allow",
  "ask",
  "deny",
]);

function isPermissionAction(value: unknown): value is PermissionActionName {
  return (
    typeof value === "string" &&
    PERMISSION_ACTIONS.has(value as PermissionActionName)
  );
}

const WHEN_KINDS = new Set([
  "any",
  "arg_equals",
  "arg_glob",
  "path_outside_workspace",
  "bash_readonly_command",
  "all_of",
  "any_of",
]);

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

/** Same matcher set the backend deserializes. Unknown kinds fail the tool. */
function parseWhen(raw: unknown): ArgMatcher | null {
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) return null;
  const when = raw as Record<string, unknown>;
  if (typeof when.kind !== "string" || !WHEN_KINDS.has(when.kind)) return null;
  switch (when.kind) {
    case "any":
    case "bash_readonly_command":
      return { kind: when.kind };
    case "arg_equals":
      if (!nonEmptyString(when.name) || typeof when.value !== "string") return null;
      return { kind: "arg_equals", name: when.name, value: when.value };
    case "arg_glob":
      if (!nonEmptyString(when.name) || !nonEmptyString(when.pattern)) return null;
      return { kind: "arg_glob", name: when.name, pattern: when.pattern };
    case "path_outside_workspace":
      if (!nonEmptyString(when.name)) return null;
      return { kind: "path_outside_workspace", name: when.name };
    case "all_of":
    case "any_of": {
      if (!Array.isArray(when.matchers) || when.matchers.length === 0) return null;
      const matchers: ArgMatcher[] = [];
      for (const child of when.matchers) {
        const parsed = parseWhen(child);
        if (!parsed) return null;
        matchers.push(parsed);
      }
      return when.kind === "all_of"
        ? { kind: "all_of", matchers }
        : { kind: "any_of", matchers };
    }
    default:
      return null;
  }
}

/** `null` means omitted or empty. `"invalid"` rejects the whole tool JSON. */
function parseRules(raw: unknown): ToolPermissionRule[] | null | "invalid" {
  if (raw === undefined || raw === null) return null;
  if (!Array.isArray(raw)) return "invalid";
  if (raw.length === 0) return null;
  const rules: ToolPermissionRule[] = [];
  const seen = new Set<string>();
  for (const rule of raw) {
    if (rule === null || typeof rule !== "object" || Array.isArray(rule)) {
      return "invalid";
    }
    const entry = rule as Record<string, unknown>;
    if (typeof entry.id !== "string") return "invalid";
    const id = entry.id.trim();
    if (!id || id === "__default" || seen.has(id)) return "invalid";
    seen.add(id);
    if (!isPermissionAction(entry.action)) return "invalid";
    const when = parseWhen(entry.when);
    if (!when) return "invalid";
    rules.push({ id, action: entry.action, when });
  }
  return rules;
}

export function parseMcpJson(
  text: string,
  expectedId?: string | null,
): SerializeResult<{ id: string; def: McpServerDefinition }> {
  try {
    const raw = JSON.parse(text) as Record<string, unknown>;
    if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
      return { skip: "invalid" };
    }
    const id = typeof raw.id === "string" ? raw.id.trim() : "";
    if (!TOOL_ID_RE.test(id)) return { skip: "invalid" };
    if (expectedId && id !== expectedId) return { skip: "invalid" };
    const command = typeof raw.command === "string" ? raw.command.trim() : "";
    const transportRaw = raw.transport;
    let transport: McpServerDefinition["transport"] = { type: "stdio" };
    if (
      transportRaw &&
      typeof transportRaw === "object" &&
      !Array.isArray(transportRaw)
    ) {
      const t = transportRaw as Record<string, unknown>;
      if (t.type === "remote") {
        if (typeof t.url !== "string" || !t.url.trim())
          return { skip: "invalid" };
        const headers =
          t.headers &&
          typeof t.headers === "object" &&
          !Array.isArray(t.headers)
            ? Object.fromEntries(
                Object.entries(t.headers as Record<string, unknown>).filter(
                  (entry): entry is [string, string] =>
                    typeof entry[1] === "string",
                ),
              )
            : {};
        transport = { type: "remote", url: t.url.trim(), headers };
      } else if (t.type === "stdio" || t.type == null) {
        transport = { type: "stdio" };
      } else {
        return { skip: "invalid" };
      }
    }
    if (transport?.type !== "remote" && !command) return { skip: "invalid" };
    const args = Array.isArray(raw.args)
      ? raw.args.filter((x): x is string => typeof x === "string")
      : [];
    const env =
      raw.env && typeof raw.env === "object" && !Array.isArray(raw.env)
        ? Object.fromEntries(
            Object.entries(raw.env as Record<string, unknown>).filter(
              (entry): entry is [string, string] =>
                typeof entry[1] === "string",
            ),
          )
        : {};
    const timeout =
      typeof raw.timeout === "number" && raw.timeout > 0 ? raw.timeout : 60;
    return {
      ok: {
        id,
        def: {
          command,
          args,
          env,
          transport: transport ?? { type: "stdio" },
          timeout,
        },
      },
    };
  } catch {
    return { skip: "invalid" };
  }
}
