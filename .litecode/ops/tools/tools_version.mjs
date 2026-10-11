import { loadRegistry, printVersionHeader } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.tools_version");
console.log(JSON.stringify({ tools_version: reg.tools_version, updated_at: reg.updated_at, count: reg.tools.length, board: ".litecode/ops/tools/board.md" }));