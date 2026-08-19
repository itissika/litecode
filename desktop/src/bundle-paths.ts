import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const LINUX_TAR_NAME = "litecode-server-linux-x64.tar.gz";
export const BUNDLED_MODEL_REL = "ibm-granite/granite-embedding-97m-multilingual-r2";

const HF_TOKENIZER = "tokenizer.json";
const HF_CONFIG = "config.json";
const ONNX_Q8Q4 = path.join("artifacts", "ort-lin-q8-emb-q4-bs128-a1.onnx");
const ONNX_LEGACY = path.join("artifacts", "ort-lin-q8-emb-q4.onnx");

export function litecodeBundlesDir(): string {
  if (process.platform === "win32") {
    const local = process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local");
    return path.join(local, "litecode", "bundles");
  }
  const home = process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share");
  return path.join(home, "litecode", "bundles");
}

export function isModelDirReady(dir: string): boolean {
  return (
    fs.existsSync(path.join(dir, HF_TOKENIZER)) &&
    fs.existsSync(path.join(dir, HF_CONFIG)) &&
    (fs.existsSync(path.join(dir, ONNX_Q8Q4)) || fs.existsSync(path.join(dir, ONNX_LEGACY)))
  );
}

/** Accept HF dir, models/ root, or bundle root. */
export function resolveHfModelDir(root: string): string | null {
  if (isModelDirReady(root)) return root;
  const nested = path.join(root, BUNDLED_MODEL_REL);
  if (isModelDirReady(nested)) return nested;
  const underModels = path.join(root, "models", BUNDLED_MODEL_REL);
  if (isModelDirReady(underModels)) return underModels;
  return null;
}

export function resolveBundledModelDir(opts: {
  sidecarRoot?: string;
  repoRoot?: string;
}): string | null {
  const envDir = process.env.LITECODE_MODEL_DIR?.trim();
  if (envDir) {
    const resolved = resolveHfModelDir(envDir);
    if (resolved) return resolved;
  }
  const bundleRoot = process.env.LITECODE_BUNDLE_ROOT?.trim();
  if (bundleRoot) {
    const resolved = resolveHfModelDir(bundleRoot);
    if (resolved) return resolved;
  }
  const fromDefault = resolveHfModelDir(litecodeBundlesDir());
  if (fromDefault) return fromDefault;
  if (opts.sidecarRoot) {
    const resolved = resolveHfModelDir(opts.sidecarRoot);
    if (resolved) return resolved;
  }
  if (opts.repoRoot) {
    const resolved = resolveHfModelDir(opts.repoRoot);
    if (resolved) return resolved;
  }
  return null;
}

export type LinuxBundleFiles = { tar: string; checksum: string };

function completeLinuxBundle(tar: string): LinuxBundleFiles | null {
  const checksum = `${tar}.sha256`;
  if (fs.existsSync(tar) && fs.existsSync(checksum)) {
    return { tar, checksum };
  }
  return null;
}

export function linuxBundleCandidateTars(opts: {
  packagedTar?: string;
  developmentTar?: string;
}): string[] {
  const candidates: string[] = [];
  if (opts.packagedTar) candidates.push(opts.packagedTar);
  const envTar = process.env.LITECODE_LINUX_BUNDLE?.trim();
  if (envTar) candidates.push(envTar);
  const bundleRoot = process.env.LITECODE_BUNDLE_ROOT?.trim();
  if (bundleRoot) candidates.push(path.join(bundleRoot, "linux", LINUX_TAR_NAME));
  candidates.push(path.join(litecodeBundlesDir(), "linux", LINUX_TAR_NAME));
  if (opts.developmentTar) candidates.push(opts.developmentTar);
  return candidates;
}

export function missingLinuxBundleMessage(candidates: string[]): string {
  const listed = candidates.map((p) => `  ${p}`).join("\n");
  return [
    "Linux server bundle (tar + .sha256) was not found.",
    "Place litecode-server-linux-x64.tar.gz and its .sha256 next to each other, then retry Open Remote.",
    "Search order:",
    listed,
    "Overrides: LITECODE_LINUX_BUNDLE (tar file) or LITECODE_BUNDLE_ROOT/linux/",
    `Default: ${path.join(litecodeBundlesDir(), "linux", LINUX_TAR_NAME)}`,
  ].join("\n");
}

export function resolveLinuxBundle(opts: {
  packagedTar?: string;
  developmentTar?: string;
}): LinuxBundleFiles {
  const candidates = linuxBundleCandidateTars(opts);
  for (const tar of candidates) {
    const found = completeLinuxBundle(tar);
    if (found) return found;
  }
  throw new Error(missingLinuxBundleMessage(candidates));
}
