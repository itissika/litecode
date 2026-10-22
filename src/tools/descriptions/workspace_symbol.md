Search symbols across the workspace. Pass `{ query }` only—no path or position. Resolves `{ symbols, message }`—each `{ name, kind, path, line, character }` (`path` required). Cap 200. Engine-not-ready rejects `ToolError`.
Example:
const hits = await tools.workspace_symbol({ query: "AgentDeps" });
if (hits.symbols.length === 0) return { ok: false, message: hits.message };
return { hit: hits.symbols[0] };
