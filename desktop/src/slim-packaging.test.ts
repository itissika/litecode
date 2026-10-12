import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { describe, it } from "node:test";

const desktopRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(desktopRoot, "..");

type ExtraResource = {
  from?: string;
  to?: string;
  filter?: string[];
};

describe("slim packaging hygiene", () => {
  it("desktop package.json extraResources excludes models and has no linux asset by default", () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(desktopRoot, "package.json"), "utf8")) as {
      build: { extraResources: ExtraResource[] };
    };
    const resources = pkg.build.extraResources;
    assert.ok(Array.isArray(resources) && resources.length > 0);

    const sidecar = resources.find((item) => item.from === "../dist/product");
    assert.ok(sidecar, "sidecar extraResource from ../dist/product");
    assert.ok(
      Array.isArray(sidecar.filter) && sidecar.filter.includes("!models/**"),
      "extraResources filter must exclude models/**",
    );

    assert.ok(
      !resources.some((item) => String(item.to ?? "").includes("linux") || String(item.from ?? "").includes("linux")),
      "default package.json must not embed the linux tar (that is -Full only)",
    );
  });

  it("package_win.ps1 defaults to slim and throws if models/linux leak into the unpacked installer", () => {
    const src = fs.readFileSync(path.join(repoRoot, "scripts", "package_win.ps1"), "utf8");
    assert.match(src, /Default = slim/);
    assert.match(src, /\[switch\]\$Full/);
    assert.match(src, /sidecar\\models/);
    assert.match(src, /throw "slim installer unexpectedly contains resources\\\$leak"/);
    assert.match(src, /\$sku = if \(\$Full\)/);
    // -Full rewrites filter to include everything; default path must not.
    assert.match(src, /\$item\.filter = @\("\*\*\/\*"\)/);
  });

  it("assemble_product.ps1 only copies models when -WithModel is set", () => {
    const src = fs.readFileSync(path.join(repoRoot, "scripts", "assemble_product.ps1"), "utf8");
    assert.match(src, /Default is slim/);
    assert.match(src, /\$IncludeModel = \[bool\]\$WithModel/);
    assert.match(src, /slim product: embed model not bundled/);
    assert.match(src, /if \(\$IncludeModel -and \(Test-Path \(Join-Path \$Root "models"\)\)\)/);
  });
});
