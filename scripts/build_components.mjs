#!/usr/bin/env node
// Stage on-demand runtime components + litecode-components.json for a release.
//
// The slim Windows installer downloads these at runtime from the GitHub
// Release of its own version (see desktop/src/components.ts). Output folder
// can also be used offline via LITECODE_COMPONENTS_BASE=<out dir>.
//
// Usage:
//   node scripts/build_components.mjs --tag v0.2.2 [--repo itissika/litecode]
//       [--out dist/components] [--model-dir models/ibm-granite/...]
//       [--linux-tar dist/linux/litecode-server-linux-x64.tar.gz]
//       [--app-version 0.2.2] [--allow-missing]
//   node scripts/build_components.mjs --print-version
//       Print Cargo.toml [package].version and exit. Single source for the
//       product version: package_linux.sh and product_version.ps1 call this.
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MODEL_REL = "ibm-granite/granite-embedding-97m-multilingual-r2";
const MODEL_ASSET = "litecode-embed-granite-97m-multilingual-r2.tar.gz";
const MODEL_VERSION = "granite-embedding-97m-multilingual-r2+ort-lin-q8-emb-q4-bs128-a1";
const LINUX_ASSET = "litecode-server-linux-x64.tar.gz";
const MANIFEST = "litecode-components.json";
// Files the runtime needs (+ license/readme for redistribution). Order is fixed for reproducible archives.
const MODEL_FILES = [
  "config.json",
  "tokenizer.json",
  "1_Pooling/config.json",
  "artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx",
  "artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx.data",
  "artifacts/ort-lin-q8-emb-q4-bs128-a1.SOURCE.json",
  "LICENSE",
  "README.md",
];
const MODEL_REQUIRED = new Set(MODEL_FILES.slice(0, 5));

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) throw new Error(`unexpected argument ${a}`);
    const key = a.slice(2);
    if (key === "allow-missing" || key === "print-version") { out[key] = true; continue; }
    const value = argv[++i];
    if (value === undefined) throw new Error(`--${key} needs a value`);
    out[key] = value;
  }
  return out;
}

/** `version` inside the `[package]` table only (other tables may carry their own). */
function cargoVersion() {
  const toml = fs.readFileSync(path.join(ROOT, "Cargo.toml"), "utf8");
  let inPackage = false;
  for (const line of toml.split(/\r?\n/)) {
    const table = /^\s*\[([^\]]+)\]\s*(#.*)?$/.exec(line);
    if (table) { inPackage = table[1].trim() === "package"; continue; }
    if (!inPackage) continue;
    const m = /^\s*version\s*=\s*"([^"]+)"\s*(#.*)?$/.exec(line);
    if (m) return m[1];
  }
  throw new Error("Could not read [package].version from Cargo.toml");
}

function sha256File(file) {
  const hash = createHash("sha256");
  const fd = fs.openSync(file, "r");
  const buf = Buffer.alloc(1 << 20);
  try {
    for (;;) {
      const n = fs.readSync(fd, buf, 0, buf.length, null);
      if (n === 0) break;
      hash.update(buf.subarray(0, n));
    }
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest("hex");
}

function octal(value, width) {
  return value.toString(8).padStart(width - 1, "0") + "\0";
}

function tarHeader(name, size) {
  if (Buffer.byteLength(name) > 100) throw new Error(`tar name too long: ${name}`);
  const h = Buffer.alloc(512, 0);
  h.write(name, 0, 100, "utf8");
  h.write(octal(0o644, 8), 100, 8, "ascii");
  h.write(octal(0, 8), 108, 8, "ascii");
  h.write(octal(0, 8), 116, 8, "ascii");
  h.write(octal(size, 12), 124, 12, "ascii");
  h.write(octal(0, 12), 136, 12, "ascii"); // fixed mtime → reproducible
  h.fill(0x20, 148, 156);
  h.write("0", 156, 1, "ascii");
  h.write("ustar\0", 257, 6, "ascii");
  h.write("00", 263, 2, "ascii");
  let sum = 0;
  for (const b of h) sum += b;
  h.write(octal(sum, 7) + " ", 148, 8, "ascii");
  return h;
}

/** Minimal deterministic ustar → gzip, streamed (model .data is ~66 MB). */
async function writeTarGz(entries, dest) {
  const gzip = zlib.createGzip({ level: 9 });
  const out = fs.createWriteStream(dest);
  const done = new Promise((resolve, reject) => {
    out.on("finish", resolve);
    out.on("error", reject);
    gzip.on("error", reject);
  });
  gzip.pipe(out);
  const write = (chunk) => new Promise((resolve) => (gzip.write(chunk) ? resolve() : gzip.once("drain", resolve)));
  for (const { name, file } of entries) {
    const size = fs.statSync(file).size;
    await write(tarHeader(name, size));
    for await (const chunk of fs.createReadStream(file, { highWaterMark: 1 << 20 })) await write(chunk);
    const pad = (512 - (size % 512)) % 512;
    if (pad) await write(Buffer.alloc(pad, 0));
  }
  await write(Buffer.alloc(1024, 0));
  gzip.end();
  await done;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args["print-version"]) {
    process.stdout.write(`${cargoVersion()}\n`);
    return;
  }
  const appVersion = (args["app-version"] ?? cargoVersion()).replace(/^v/, "");
  const tag = args.tag ?? `v${appVersion}`;
  const repo = args.repo ?? process.env.GITHUB_REPOSITORY ?? "itissika/litecode";
  const outDir = path.resolve(args.out ?? path.join(ROOT, "dist", "components"));
  const modelDir = path.resolve(args["model-dir"] ?? path.join(ROOT, "models", MODEL_REL));
  const linuxTar = path.resolve(args["linux-tar"] ?? path.join(ROOT, "dist", "linux", LINUX_ASSET));
  const allowMissing = Boolean(args["allow-missing"]);
  const urlFor = (asset) => `https://github.com/${repo}/releases/download/${tag}/${asset}`;

  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(outDir, { recursive: true });
  const components = [];

  const missingModel = [...MODEL_REQUIRED].filter((rel) => !fs.existsSync(path.join(modelDir, rel)));
  if (missingModel.length === 0) {
    const entries = MODEL_FILES.filter((rel) => fs.existsSync(path.join(modelDir, rel))).map((rel) => ({
      name: rel,
      file: path.join(modelDir, rel),
    }));
    const dest = path.join(outDir, MODEL_ASSET);
    console.log(`==> packing embed model (${entries.length} files) -> ${dest}`);
    await writeTarGz(entries, dest);
    components.push({
      id: "embed-model",
      version: MODEL_VERSION,
      platform: "any",
      file: MODEL_ASSET,
      url: urlFor(MODEL_ASSET),
      size: fs.statSync(dest).size,
      sha256: sha256File(dest),
      format: "tar.gz",
      app_version_range: { min: appVersion },
    });
  } else if (allowMissing) {
    console.warn(`!! embed model incomplete at ${modelDir} (missing ${missingModel.join(", ")}); skipped`);
  } else {
    throw new Error(`embed model incomplete at ${modelDir}: missing ${missingModel.join(", ")}`);
  }

  if (fs.existsSync(linuxTar)) {
    const dest = path.join(outDir, LINUX_ASSET);
    console.log(`==> copying Linux server bundle -> ${dest}`);
    fs.copyFileSync(linuxTar, dest);
    const sha256 = sha256File(dest);
    const pinned = `${linuxTar}.sha256`;
    if (fs.existsSync(pinned)) {
      const expected = fs.readFileSync(pinned, "utf8").trim().split(/\s+/)[0]?.toLowerCase();
      if (expected && expected !== sha256) throw new Error(`Linux tar does not match ${pinned}`);
    }
    components.push({
      id: "linux-server",
      version: appVersion,
      platform: "linux-x64",
      file: LINUX_ASSET,
      url: urlFor(LINUX_ASSET),
      size: fs.statSync(dest).size,
      sha256,
      format: "file",
      // The remote install dir is keyed by app version; the server must match exactly.
      app_version_range: { min: appVersion, max: appVersion },
    });
  } else if (allowMissing) {
    console.warn(`!! Linux tar missing at ${linuxTar}; skipped`);
  } else {
    throw new Error(`Linux server bundle missing at ${linuxTar} (run scripts/package_linux.sh)`);
  }

  const manifest = {
    schema: 1,
    app_version: appVersion,
    tag,
    repo,
    generated_at: new Date().toISOString(),
    components,
  };
  fs.writeFileSync(path.join(outDir, MANIFEST), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  console.log(`==> ${MANIFEST}:`);
  for (const c of components) {
    console.log(`  ${c.id.padEnd(13)} ${(c.size / 1024 / 1024).toFixed(1).padStart(6)} MB  ${c.sha256}  ${c.file}`);
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
