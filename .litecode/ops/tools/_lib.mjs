import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
export function opsRoot(fromMetaUrl) {
  return path.join(path.dirname(fileURLToPath(fromMetaUrl)), "..");
}
export function loadRegistry(fromMetaUrl) {
  const regPath = path.join(path.dirname(fileURLToPath(fromMetaUrl)), "registry.json");
  return { regPath, reg: JSON.parse(fs.readFileSync(regPath, "utf8")) };
}
export function printVersionHeader(reg, toolId) {
  const t = (reg.tools || []).find((x) => x.id === toolId);
  const line = `LITECODE_OPS tools_version=${reg.tools_version} tool=${toolId} tool_version=${t ? t.version : "?"} updated_at=${reg.updated_at}`;
  console.log(line);
  return line;
}
export function recordTrace(ops, wp, toolId, toolsVersion) {
  if (!wp) return;
  const p = path.join(ops, "tool_traces.jsonl");
  fs.appendFileSync(p, JSON.stringify({ ts: new Date().toISOString(), wp, tool: toolId, tools_version: toolsVersion }) + "\n");
}