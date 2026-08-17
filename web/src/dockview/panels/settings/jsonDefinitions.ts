import type {
  CustomToolDefinition,
  McpServerDefinition,
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
    if (schemaRaw === null || typeof schemaRaw !== "object" || Array.isArray(schemaRaw)) {
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
    if (!Array.isArray(required) || required.some((x) => typeof x !== "string")) {
      return { skip: "invalid" };
    }
    const args = Array.isArray(raw.args)
      ? raw.args.filter((x): x is string => typeof x === "string")
      : [];
    const timeout =
      typeof raw.timeout === "number" && raw.timeout > 0 ? raw.timeout : 120;
    return {
      ok: {
        name,
        description: typeof raw.description === "string" ? raw.description.trim() : "",
        command,
        args,
        timeout,
        schema: {
          type: typeof schema.type === "string" ? schema.type : "object",
          properties: properties as Record<string, unknown>,
          required: required as string[],
        },
      },
    };
  } catch {
    return { skip: "invalid" };
  }
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
    if (transportRaw && typeof transportRaw === "object" && !Array.isArray(transportRaw)) {
      const t = transportRaw as Record<string, unknown>;
      if (t.type === "remote") {
        if (typeof t.url !== "string" || !t.url.trim()) return { skip: "invalid" };
        const headers =
          t.headers && typeof t.headers === "object" && !Array.isArray(t.headers)
            ? Object.fromEntries(
                Object.entries(t.headers as Record<string, unknown>).filter(
                  (entry): entry is [string, string] => typeof entry[1] === "string",
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
              (entry): entry is [string, string] => typeof entry[1] === "string",
            ),
          )
        : {};
    return {
      ok: {
        id,
        def: {
          command,
          args,
          env,
          transport: transport ?? { type: "stdio" },
        },
      },
    };
  } catch {
    return { skip: "invalid" };
  }
}
