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

  it("package_win.ps1 throws if models/linux leak into the unpacked installer", () => {
    const src = fs.readFileSync(path.join(repoRoot, "scripts", "package_win.ps1"), "utf8");
    assert.match(src, /\[switch\]\$Full/);
    assert.match(src, /sidecar\\models/);
    assert.match(src, /throw "slim installer unexpectedly contains resources\\\$leak"/);
    assert.match(src, /\$sku = if \(\$Full\)/);
    // -Full rewrites filter to include everything; default path must not.
    assert.match(src, /\$item\.filter = @\("\*\*\/\*"\)/);
  });

  it("assemble_product.ps1 gates model copy on -WithModel", () => {
    const src = fs.readFileSync(path.join(repoRoot, "scripts", "assemble_product.ps1"), "utf8");
    // Structural: IncludeModel derives from -WithModel; models Copy-Item only inside that gate.
    assert.match(src, /\$IncludeModel = \[bool\]\$WithModel/);
    assert.match(
      src,
      /if \(\$IncludeModel -and \(Test-Path \(Join-Path \$Root "models"\)\)\)[\s\S]*?Copy-Item -Recurse -Force \(Join-Path \$Root "models\\\*"\)/,
    );
  });

  it("sh/ps1 script pairs cross-reference each other and keep the release remap in sync", () => {
    const read = (name: string) => fs.readFileSync(path.join(repoRoot, "scripts", name), "utf8");
    const pairs: Array<[string, string]> = [
      ["assemble_product.sh", "assemble_product.ps1"],
      ["smoke_product.sh", "smoke_product.ps1"],
      ["serve.sh", "serve_win.ps1"],
    ];
    for (const [sh, ps1] of pairs) {
      assert.ok(read(sh).includes(`Paired with scripts/${ps1}`), `${sh} header must point at ${ps1}`);
      assert.ok(read(ps1).includes(`Paired with scripts/${sh}`), `${ps1} header must point at ${sh}`);
    }
    const assembleSh = read("assemble_product.sh");
    const assemblePs1 = read("assemble_product.ps1");
    for (const needle of ["--remap-path-prefix=", "=/rustc/registry", "=/rustc/git", "RUSTFLAGS (release remap)"]) {
      assert.ok(assembleSh.includes(needle), `assemble_product.sh missing ${needle}`);
      assert.ok(assemblePs1.includes(needle), `assemble_product.ps1 missing ${needle}`);
    }
  });
});
