import fs from "node:fs";
import path from "node:path";
import { app } from "electron";

/** Directory that contains litecode.exe + web/dist + models (+ dlls). */
export function sidecarRoot(): string {
  const override = process.env.LITECODE_SIDECAR_DIR;
  if (override && fs.existsSync(override)) {
    return path.resolve(override);
  }

  if (app.isPackaged) {
    return path.join(process.resourcesPath, "sidecar");
  }

  // Dev: prefer repo dist/product, then target/debug next to desktop/
  const repoRoot = path.resolve(__dirname, "..", "..");
  const product = path.join(repoRoot, "dist", "product");
  if (fs.existsSync(path.join(product, "litecode.exe")) || fs.existsSync(path.join(product, "litecode"))) {
    return product;
  }
  const debug = path.join(repoRoot, "target", "debug");
  if (fs.existsSync(path.join(debug, "litecode.exe")) || fs.existsSync(path.join(debug, "litecode"))) {
    return debug;
  }
  const release = path.join(repoRoot, "target", "release");
  if (fs.existsSync(path.join(release, "litecode.exe")) || fs.existsSync(path.join(release, "litecode"))) {
    return release;
  }
  return product;
}

export function litecodeBinary(root: string): string {
  const win = path.join(root, "litecode.exe");
  if (fs.existsSync(win)) return win;
  const unix = path.join(root, "litecode");
  if (fs.existsSync(unix)) return unix;
  throw new Error(`litecode binary not found under ${root}`);
}
