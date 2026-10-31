import { readFileSync } from "node:fs";
import { relative } from "node:path";

import { describe, expect, it } from "vitest";

import { collectTsFiles, scanNeedles, WEB_SRC } from "../test/deathListGate";

describe("native view occlusion contract", () => {
  it("keeps the legacy global obscure path out of the renderer", () => {
    const violations = scanNeedles(collectTsFiles(), [
      "browserSetObscured",
      "installBrowserChromeObscure",
      "pushBrowserObscure",
    ]);
    expect(violations).toEqual([]);
  });

  it("marks every createPortal surface as an overlay", () => {
    const missing = collectTsFiles()
      .filter((file) => readFileSync(file, "utf8").includes("createPortal("))
      .filter(
        (file) =>
          !readFileSync(file, "utf8").includes("data-litecode-overlay"),
      )
      .map((file) => relative(WEB_SRC, file));
    expect(missing).toEqual([]);
  });
});
