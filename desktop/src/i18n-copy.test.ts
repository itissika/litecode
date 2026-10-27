import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import { pathToFileURL } from "node:url";

type CopyMod = {
  copyI18nLocales: (
    srcDir: string,
    destDir: string,
  ) => Array<{ from: string; to: string }>;
};

/** tsc (module=commonjs) rewrites `await import(url)` to require(); keep native ESM load. */
const importEsm = new Function(
  "url",
  "return import(url)",
) as (url: string) => Promise<CopyMod>;

describe("copy-i18n-locales", () => {
  it("copies locale JSON files into the destination directory", async () => {
    const script = path.resolve(__dirname, "../scripts/copy-i18n-locales.mjs");
    const mod = await importEsm(pathToFileURL(script).href);

    const root = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-i18n-copy-"));
    const srcDir = path.join(root, "locales");
    const destDir = path.join(root, "dist", "i18n");
    fs.mkdirSync(srcDir, { recursive: true });
    fs.writeFileSync(path.join(srcDir, "en.json"), JSON.stringify({ hub: { home: { title: "Workspace" } } }));
    fs.writeFileSync(
      path.join(srcDir, "zh-CN.json"),
      JSON.stringify({ hub: { home: { title: "工作区" } } }),
    );

    try {
      const copied = mod.copyI18nLocales(srcDir, destDir);
      assert.equal(copied.length, 2);
      assert.equal(fs.existsSync(path.join(destDir, "en.json")), true);
      assert.equal(fs.existsSync(path.join(destDir, "zh-CN.json")), true);
      const en = JSON.parse(fs.readFileSync(path.join(destDir, "en.json"), "utf8")) as {
        hub: { home: { title: string } };
      };
      assert.equal(en.hub.home.title, "Workspace");
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it("package.json build and test scripts hook the copy step", () => {
    const pkg = JSON.parse(
      fs.readFileSync(path.resolve(__dirname, "../package.json"), "utf8"),
    ) as { scripts: Record<string, string> };
    assert.match(pkg.scripts.build, /copy-i18n-locales\.mjs/);
    assert.match(pkg.scripts.test, /copy-i18n-locales\.mjs/);
  });
});
