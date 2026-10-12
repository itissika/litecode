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
  litecodeBundlesDir,
  resolveBundledModelDir,
  resolveHfModelDir,
  linuxBundleCandidateTars,
  resolveLinuxBundle,
} from "./bundle-paths";

const saved = {
  MODEL_DIR: process.env.LITECODE_MODEL_DIR,
  LINUX_BUNDLE: process.env.LITECODE_LINUX_BUNDLE,
  BUNDLE_ROOT: process.env.LITECODE_BUNDLE_ROOT,
  XDG: process.env.XDG_DATA_HOME,
};

const temps: string[] = [];

afterEach(() => {
  restore("LITECODE_MODEL_DIR", saved.MODEL_DIR);
  restore("LITECODE_LINUX_BUNDLE", saved.LINUX_BUNDLE);
  restore("LITECODE_BUNDLE_ROOT", saved.BUNDLE_ROOT);
  restore("XDG_DATA_HOME", saved.XDG);
  for (const dir of temps.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function restore(name: string, value: string | undefined): void {
  if (value === undefined) delete process.env[name];
  else process.env[name] = value;
}

function tmp(prefix: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  temps.push(dir);
  return dir;
}

function writeReadyModel(root: string, tag = "x"): string {
  const dir = path.join(root, "models", BUNDLED_MODEL_REL);
  fs.mkdirSync(path.join(dir, "artifacts"), { recursive: true });
  fs.writeFileSync(path.join(dir, "tokenizer.json"), tag);
  fs.writeFileSync(path.join(dir, "config.json"), "{}");
  fs.writeFileSync(path.join(dir, "artifacts", "ort-lin-q8-emb-q4-bs128-a1.onnx"), "onnx");
  return dir;
}

function writeLinuxPair(dir: string, tag = "tar"): string {
  fs.mkdirSync(dir, { recursive: true });
  const tar = path.join(dir, LINUX_TAR_NAME);
  fs.writeFileSync(tar, tag);
  fs.writeFileSync(`${tar}.sha256`, `${tag}-sum`);
  return tar;
}

function clearModelEnv(): void {
  delete process.env.LITECODE_MODEL_DIR;
  delete process.env.LITECODE_BUNDLE_ROOT;
}

function clearLinuxEnv(): void {
  delete process.env.LITECODE_LINUX_BUNDLE;
  delete process.env.LITECODE_BUNDLE_ROOT;
}

describe("bundle-paths", () => {
  it("detects a ready HF model directory", () => {
    const root = tmp("litecode-model-");
    const dir = writeReadyModel(root);
    assert.equal(isModelDirReady(dir), true);
    assert.equal(resolveHfModelDir(root), dir);
  });

  it("resolves LITECODE_MODEL_DIR over sidecar models", () => {
    const dir = writeReadyModel(tmp("litecode-model-env-"));
    process.env.LITECODE_MODEL_DIR = dir;
    const sidecar = tmp("litecode-sidecar-");
    assert.equal(resolveBundledModelDir({ sidecarRoot: sidecar }), dir);
  });

  it("resolves linux tar from LITECODE_LINUX_BUNDLE", () => {
    const tar = writeLinuxPair(tmp("litecode-linux-"));
    process.env.LITECODE_LINUX_BUNDLE = tar;
    const found = resolveLinuxBundle({});
    assert.equal(found.tar, tar);
    assert.equal(found.checksum, `${tar}.sha256`);
  });

  it("resolves linux tar from LITECODE_BUNDLE_ROOT/linux", () => {
    const root = tmp("litecode-bundle-");
    const tar = writeLinuxPair(path.join(root, "linux"));
    delete process.env.LITECODE_LINUX_BUNDLE;
    process.env.LITECODE_BUNDLE_ROOT = root;
    const found = resolveLinuxBundle({});
    assert.equal(found.tar, tar);
  });

  it("fingerprints a ready model directory stably", () => {
    const dir = writeReadyModel(tmp("litecode-model-hash-"));
    const a = hashModelDir(dir);
    const b = hashModelDir(dir);
    assert.equal(a, b);
    assert.match(a, /^[a-f0-9]{64}$/);
    fs.appendFileSync(path.join(dir, "tokenizer.json"), "x");
    assert.notEqual(hashModelDir(dir), a);
  });
});

describe("model resolve order env → bundles → sidecar → repo", () => {
  it("locks the full precedence when every source is present", () => {
    clearModelEnv();
    process.env.XDG_DATA_HOME = tmp("lc-xdg-");

    const envRoot = tmp("lc-env-");
    const envDir = writeReadyModel(envRoot, "env");
    const bundleRoot = tmp("lc-broot-");
    const fromBundleRoot = writeReadyModel(bundleRoot, "broot");
    const fromDefault = writeReadyModel(litecodeBundlesDir(), "default");
    const sidecarRoot = tmp("lc-side-");
    const fromSidecar = writeReadyModel(sidecarRoot, "side");
    const repoRoot = tmp("lc-repo-");
    const fromRepo = writeReadyModel(repoRoot, "repo");

    process.env.LITECODE_MODEL_DIR = envDir;
    process.env.LITECODE_BUNDLE_ROOT = bundleRoot;
    assert.equal(
      resolveBundledModelDir({ sidecarRoot, repoRoot }),
      envDir,
      "env wins over bundles/sidecar/repo",
    );

    delete process.env.LITECODE_MODEL_DIR;
    assert.equal(
      resolveBundledModelDir({ sidecarRoot, repoRoot }),
      fromBundleRoot,
      "LITECODE_BUNDLE_ROOT wins over default bundles/sidecar/repo",
    );

    delete process.env.LITECODE_BUNDLE_ROOT;
    assert.equal(
      resolveBundledModelDir({ sidecarRoot, repoRoot }),
      fromDefault,
      "default bundles wins over sidecar/repo",
    );

    // Drop the default-bundles model so sidecar can win.
    fs.rmSync(fromDefault, { recursive: true, force: true });
    assert.equal(resolveBundledModelDir({ sidecarRoot, repoRoot }), fromSidecar, "sidecar wins over repo");
    assert.equal(resolveBundledModelDir({ repoRoot }), fromRepo, "repo is the last fallback");
    assert.equal(resolveBundledModelDir({}), null);
  });

  it("earlier model sources win pairwise over later ones", () => {
    clearModelEnv();
    process.env.XDG_DATA_HOME = tmp("lc-xdg-pair-");

    const envDir = writeReadyModel(tmp("lc-pair-env-"), "env");
    const bundleRoot = tmp("lc-pair-broot-");
    const brootDir = writeReadyModel(bundleRoot, "broot");
    const defaultDir = writeReadyModel(litecodeBundlesDir(), "default");
    const sidecarRoot = tmp("lc-pair-side-");
    const sideDir = writeReadyModel(sidecarRoot, "side");
    const repoRoot = tmp("lc-pair-repo-");
    const repoDir = writeReadyModel(repoRoot, "repo");

    process.env.LITECODE_MODEL_DIR = envDir;
    process.env.LITECODE_BUNDLE_ROOT = bundleRoot;
    assert.equal(resolveBundledModelDir({ sidecarRoot, repoRoot }), envDir);

    delete process.env.LITECODE_MODEL_DIR;
    assert.equal(resolveBundledModelDir({ sidecarRoot, repoRoot }), brootDir);

    delete process.env.LITECODE_BUNDLE_ROOT;
    assert.equal(resolveBundledModelDir({ sidecarRoot, repoRoot }), defaultDir);

    fs.rmSync(defaultDir, { recursive: true, force: true });
    assert.equal(resolveBundledModelDir({ sidecarRoot, repoRoot }), sideDir);
    assert.equal(resolveBundledModelDir({ repoRoot }), repoDir);
  });
});

describe("linux resolve order packaged → env → bundles → repo", () => {
  it("earlier linux candidates win when several complete pairs exist", () => {
    clearLinuxEnv();
    const xdg = tmp("lc-linux-xdg-");
    process.env.XDG_DATA_HOME = xdg;

    const packaged = writeLinuxPair(tmp("lc-pack-"), "packaged");
    const envTar = writeLinuxPair(tmp("lc-env-tar-"), "env");
    const bundleRoot = tmp("lc-linux-broot-");
    const brootTar = writeLinuxPair(path.join(bundleRoot, "linux"), "broot");
    const defaultTar = writeLinuxPair(path.join(litecodeBundlesDir(), "linux"), "default");
    const development = writeLinuxPair(tmp("lc-dev-linux-"), "dev");

    process.env.LITECODE_LINUX_BUNDLE = envTar;
    process.env.LITECODE_BUNDLE_ROOT = bundleRoot;

    assert.equal(
      resolveLinuxBundle({ packagedTar: packaged, developmentTar: development }).tar,
      packaged,
      "packaged wins over env/bundles/repo",
    );
    assert.equal(
      resolveLinuxBundle({ developmentTar: development }).tar,
      envTar,
      "env wins over bundles/repo when no packaged",
    );

    delete process.env.LITECODE_LINUX_BUNDLE;
    assert.equal(
      resolveLinuxBundle({ developmentTar: development }).tar,
      brootTar,
      "LITECODE_BUNDLE_ROOT/linux wins over default bundles/repo",
    );

    delete process.env.LITECODE_BUNDLE_ROOT;
    assert.equal(
      resolveLinuxBundle({ developmentTar: development }).tar,
      defaultTar,
      "default bundles wins over development",
    );

    fs.rmSync(defaultTar, { force: true });
    fs.rmSync(`${defaultTar}.sha256`, { force: true });
    assert.equal(
      resolveLinuxBundle({ developmentTar: development }).tar,
      development,
      "development is the last fallback",
    );
  });

  it("candidate list order matches packaged → env → bundle-root → default → development", () => {
    clearLinuxEnv();
    const xdg = tmp("lc-cand-xdg-");
    process.env.XDG_DATA_HOME = xdg;
    const packaged = path.join(tmp("lc-cand-pack-"), LINUX_TAR_NAME);
    const envTar = path.join(tmp("lc-cand-env-"), LINUX_TAR_NAME);
    const bundleRoot = tmp("lc-cand-broot-");
    const development = path.join(tmp("lc-cand-dev-"), LINUX_TAR_NAME);
    process.env.LITECODE_LINUX_BUNDLE = envTar;
    process.env.LITECODE_BUNDLE_ROOT = bundleRoot;
    assert.deepEqual(
      linuxBundleCandidateTars({ packagedTar: packaged, developmentTar: development }),
      [
        packaged,
        envTar,
        path.join(bundleRoot, "linux", LINUX_TAR_NAME),
        path.join(litecodeBundlesDir(), "linux", LINUX_TAR_NAME),
        development,
      ],
    );
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
    clearModelEnv();
    clearLinuxEnv();
    const repo = tmp("lc-repo-");
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
  });
});

function linuxCandidates(isPackaged: boolean, desktop: string): string[] {
  return linuxBundleCandidateTars({ developmentTar: devLinuxBundleTar(isPackaged, desktop) });
}
