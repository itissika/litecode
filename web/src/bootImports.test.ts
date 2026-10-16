import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(join(src, rel), "utf8");
const staticImports = (text: string) =>
  [...text.matchAll(/^import\s+(?:[^;]*?from\s+)?"([^"]+)";/gm)].map((m) => m[1]);

describe("WP-170 boot imports", () => {
  it("S3: main.tsx and the panel registry do not pull monaco statically", () => {
    expect(staticImports(read("main.tsx"))).not.toContain("./lib/monaco");
    const registry = staticImports(read("dockview/config/registry.ts"));
    expect(registry).not.toContain("../panels/EditorPanel");
    expect(read("dockview/config/lazyEditorPanel.tsx")).toContain(
      'import("../panels/EditorPanel")',
    );
  });

  it("S3: EditorPanel configures the local monaco bundle before anything else", () => {
    expect(staticImports(read("dockview/panels/EditorPanel.tsx"))[0]).toBe(
      "../../lib/monaco",
    );
  });

  it("S4: first paint loads only the 400 font faces", () => {
    const fonts = staticImports(read("main.tsx")).filter((p) =>
      p.startsWith("@fontsource/"),
    );
    expect(fonts).toEqual([
      "@fontsource/inter/400.css",
      "@fontsource/jetbrains-mono/400.css",
    ]);
    expect(staticImports(read("fontsDeferred.ts")).sort()).toEqual([
      "@fontsource/inter/200.css",
      "@fontsource/inter/300.css",
      "@fontsource/inter/500.css",
      "@fontsource/jetbrains-mono/300.css",
      "@fontsource/jetbrains-mono/500.css",
    ]);
    expect(read("main.tsx")).toContain('import("./fontsDeferred")');
  });
});
