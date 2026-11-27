import fs from "node:fs";
import path from "node:path";

import { app } from "electron";

import { devRepoRoot } from "./bundle-paths";

const DEV_BINARY_DIRS = [
  path.join("dist", "product"),
  path.join("target", "debug"),
  path.join("target", "release"),
];

function findBinaryIn(dir: string): string | null {
  for (const name of ["litecode.exe", "litecode"]) {
    const candidate = path.join(dir, name);
    if (fs.existsSync(candidate)) return candidate;
  }
  return null;
}

/** Directory that contains litecode.exe + web/dist + models (+ dlls). */
export function sidecarRoot(): string {
  const override = process.env.LITECODE_SIDECAR_DIR;
  if (override && fs.existsSync(override)) {
    return path.resolve(override);
  }

  if (app.isPackaged) {
    return path.join(process.resourcesPath, "sidecar");
  }

  // Dev layout: repo dist/product, then target/debug, then target/release.
  const repoRoot = devRepoRoot(app.isPackaged, path.resolve(__dirname, ".."));
  if (!repoRoot) return path.join(process.resourcesPath, "sidecar");
  const probes = DEV_BINARY_DIRS.map((rel) => path.join(repoRoot, rel));
  for (const dir of probes) {
    if (findBinaryIn(dir)) return dir;
  }
  return probes[0];
}

export function litecodeBinary(root: string): string {
  const bin = findBinaryIn(root);
  if (!bin) throw new Error(`litecode binary not found under ${root}`);
  return bin;
}
