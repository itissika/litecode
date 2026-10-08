import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const SRC = path.resolve(import.meta.dirname, "../..");

const FORBIDDEN = [
  /\.addPanel\s*\(/,
  /\.addGroup\s*\(/,
  /\.moveTo\s*\(\s*\{/,
  /\.addPopoutGroup\s*\(/,
];

function files(dir: string): string[] {
  const found: string[] = [];
  for (const name of readdirSync(dir)) {
    if (name === "workbench" || name === "node_modules") continue;
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) {
      found.push(...files(full));
      continue;
    }
    if (name.endsWith(".ts") || name.endsWith(".tsx")) found.push(full);
  }
  return found;
}

describe("workbench guard", () => {
  it("keeps Dockview mutations inside the panel manager", () => {
    const hits: string[] = [];
    for (const file of files(SRC)) {
      const text = readFileSync(file, "utf8");
      for (const pattern of FORBIDDEN) {
        if (pattern.test(text)) hits.push(`${path.relative(SRC, file)} ${pattern.source}`);
      }
    }
    expect(hits).toEqual([]);
  });
});
