import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, it } from "node:test";

import {
  BUNDLED_MODEL_REL,
  LINUX_TAR_NAME,
  devLinuxBundleTar,
  devRepoRoot,
  hashModelDir,
  isModelDirReady,
  resolveBundledModelDir,
  resolveHfModelDir,
  linuxBundleCandidateTars,
  resolveLinuxBundle,
} from "./bundle-paths";

const saved = {
  MODEL_DIR: process.env.LITECODE_MODEL_DIR,
  LINUX_BUNDLE: process.env.LITECODE_LINUX_BUNDLE,
  BUNDLE_ROOT: process.env.LITECODE_BUNDLE_ROOT,
};

afterEach(() => {
  restore("LITECODE_MODEL_DIR", saved.MODEL_DIR);
  restore("LITECODE_LINUX_BUNDLE", saved.LINUX_BUNDLE);
  restore("LITECODE_BUNDLE_ROOT", saved.BUNDLE_ROOT);
});

function restore(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}

function writeReadyModel(root: string): string {
  const dir = path.join(root, "models", BUNDLED_MODEL_REL);
  fs.mkdirSync(path.join(dir, "artifacts"), { recursive: true });
  fs.writeFileSync(path.join(dir, "tokenizer.json"), "{}");
  fs.writeFileSync(path.join(dir, "config.json"), "{}");
  fs.writeFileSync(path.join(dir, "artifacts", "ort-lin-q8-emb-q4-bs128-a1.onnx"), "onnx");
  return dir;
}

describe("bundle-paths", () => {
  it("detects a ready HF model directory", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-model-"));
    const dir = writeReadyModel(tmp);
    assert.equal(isModelDirReady(dir), true);
    assert.equal(resolveHfModelDir(tmp), dir);
  });

  it("resolves LITECODE_MODEL_DIR over sidecar models", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-model-env-"));
    const dir = writeReadyModel(tmp);
    process.env.LITECODE_MODEL_DIR = dir;
    const sidecar = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-sidecar-"));
    assert.equal(resolveBundledModelDir({ sidecarRoot: sidecar }), dir);
  });

  it("resolves linux tar from LITECODE_LINUX_BUNDLE", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-linux-"));
    const tar = path.join(tmp, LINUX_TAR_NAME);
    fs.writeFileSync(tar, "tar");
    fs.writeFileSync(`${tar}.sha256`, "deadbeef");
    process.env.LITECODE_LINUX_BUNDLE = tar;
    const found = resolveLinuxBundle({});
    assert.equal(found.tar, tar);
    assert.equal(found.checksum, `${tar}.sha256`);
  });

  it("resolves linux tar from LITECODE_BUNDLE_ROOT/linux", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-bundle-"));
    const linux = path.join(tmp, "linux");
    fs.mkdirSync(linux);
    const tar = path.join(linux, LINUX_TAR_NAME);
    fs.writeFileSync(tar, "tar");
    fs.writeFileSync(`${tar}.sha256`, "deadbeef");
    delete process.env.LITECODE_LINUX_BUNDLE;
    process.env.LITECODE_BUNDLE_ROOT = tmp;
    const found = resolveLinuxBundle({});
    assert.equal(found.tar, tar);
  });

  it("fingerprints a ready model directory stably", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-model-hash-"));
    const dir = writeReadyModel(tmp);
    const a = hashModelDir(dir);
    const b = hashModelDir(dir);
    assert.equal(a, b);
    assert.match(a, /^[a-f0-9]{64}$/);
    fs.appendFileSync(path.join(dir, "tokenizer.json"), "x");
    assert.notEqual(hashModelDir(dir), a);
  });
});

describe("dev-only repo fallbacks", () => {
  it("resolve to the checkout in dev and to nothing in a packaged app", () => {
    const desktop = path.join(os.tmpdir(), "repo", "desktop");
    assert.equal(devRepoRoot(false, desktop), path.join(os.tmpdir(), "repo"));
    assert.equal(devLinuxBundleTar(false, desktop), path.join(os.tmpdir(), "repo", "dist", "linux", LINUX_TAR_NAME));
    assert.equal(devRepoRoot(true, desktop), undefined);
    assert.equal(devLinuxBundleTar(true, desktop), undefined);
  });

  it("a packaged app never picks a model or Linux tar out of a source tree", () => {
    delete process.env.LITECODE_MODEL_DIR;
    delete process.env.LITECODE_BUNDLE_ROOT;
    delete process.env.LITECODE_LINUX_BUNDLE;
    const repo = fs.mkdtempSync(path.join(os.tmpdir(), "lc-repo-"));
    try {
      const desktop = path.join(repo, "desktop");
      const model = path.join(repo, "models", ...BUNDLED_MODEL_REL.split("/"));
      fs.mkdirSync(path.join(model, "artifacts"), { recursive: true });
      for (const f of ["config.json", "tokenizer.json", path.join("artifacts", "ort-lin-q8-emb-q4-bs128-a1.onnx")]) {
        fs.writeFileSync(path.join(model, f), "x");
      }
      const tar = path.join(repo, "dist", "linux", LINUX_TAR_NAME);
      fs.mkdirSync(path.dirname(tar), { recursive: true });
      fs.writeFileSync(tar, "t");
      fs.writeFileSync(`${tar}.sha256`, "0");

      // Dev: the checkout's models/ is a valid fallback; packaged: no repo root at all.
      assert.equal(resolveHfModelDir(devRepoRoot(false, desktop)!), model);
      assert.equal(devRepoRoot(true, desktop), undefined);
      const packagedCandidates = linuxCandidates(true, desktop);
      assert.ok(!packagedCandidates.some((c) => path.resolve(c).startsWith(path.resolve(repo))));
      assert.ok(linuxCandidates(false, desktop).includes(tar));
    } finally {
      fs.rmSync(repo, { recursive: true, force: true });
    }
  });
});

function linuxCandidates(isPackaged: boolean, desktop: string): string[] {
  return linuxBundleCandidateTars({ developmentTar: devLinuxBundleTar(isPackaged, desktop) });
}
