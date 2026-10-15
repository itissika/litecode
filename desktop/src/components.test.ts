import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, it } from "node:test";
import zlib from "node:zlib";

import { BUNDLED_MODEL_REL, LINUX_TAR_NAME, isModelDirReady } from "./bundle-paths";
import {
  COMPONENTS_MANIFEST_NAME,
  ComponentManager,
  cleanupComponentTmp,
  compareVersions,
  componentPartPath,
  componentPayloadLocation,
  DownloadControlError,
  extractTarGz,
  installedLinuxTar,
  installedModelDir,
  isDownloadControlError,
  parseManifest,
  progressPercent,
  readInstallRecord,
  resolveComponentSource,
  selectComponent,
  SpeedTracker,
  versionInRange,
  type ComponentProgressEvent,
  type ComponentsManifest,
} from "./components";

const BUILD_SCRIPT = path.resolve(__dirname, "..", "..", "scripts", "build_components.mjs");
const temps: string[] = [];

function tmp(prefix: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  temps.push(dir);
  return dir;
}

afterEach(() => {
  for (const dir of temps.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});

function sha(buf: Buffer): string {
  return createHash("sha256").update(buf).digest("hex");
}

function fixtureModel(root: string): string {
  const dir = path.join(root, "model");
  fs.mkdirSync(path.join(dir, "artifacts"), { recursive: true });
  fs.mkdirSync(path.join(dir, "1_Pooling"), { recursive: true });
  fs.writeFileSync(path.join(dir, "config.json"), '{"a":1}');
  fs.writeFileSync(path.join(dir, "tokenizer.json"), "t".repeat(70_000));
  fs.writeFileSync(path.join(dir, "1_Pooling", "config.json"), '{"pooling_mode_cls_token":true}');
  fs.writeFileSync(path.join(dir, "artifacts", "ort-lin-q8-emb-q4-bs128-a1.onnx"), "onnx");
  fs.writeFileSync(path.join(dir, "artifacts", "ort-lin-q8-emb-q4-bs128-a1.onnx.data"), Buffer.alloc(1300, 7));
  return dir;
}

/** Stage components with the real release script into `out`. */
function buildComponents(appVersion = "1.2.3"): { out: string; manifest: ComponentsManifest } {
  const root = tmp("lc-comp-src-");
  const model = fixtureModel(root);
  const tar = path.join(root, LINUX_TAR_NAME);
  fs.writeFileSync(tar, Buffer.from("linux-server-bytes"));
  const out = path.join(root, "components");
  execFileSync(process.execPath, [
    BUILD_SCRIPT,
    "--app-version", appVersion,
    "--tag", `v${appVersion}`,
    "--repo", "itissika/litecode",
    "--out", out,
    "--model-dir", model,
    "--linux-tar", tar,
  ], { stdio: "pipe" });
  const manifest = parseManifest(JSON.parse(fs.readFileSync(path.join(out, COMPONENTS_MANIFEST_NAME), "utf8")));
  return { out, manifest };
}

function manager(opts: {
  bundles: string;
  env: NodeJS.ProcessEnv;
  appVersion?: string;
  fetchImpl?: typeof fetch;
  events?: ComponentProgressEvent[];
  locateLinux?: () => string | null;
}): ComponentManager {
  return new ComponentManager({
    appVersion: opts.appVersion ?? "1.2.3",
    bundlesDir: opts.bundles,
    env: opts.env,
    fetchImpl: opts.fetchImpl,
    emit: (event) => opts.events?.push(event),
    locate: {
      "embed-model": () => (isModelDirReady(installedModelDir(opts.bundles)) ? installedModelDir(opts.bundles) : null),
      "linux-server": opts.locateLinux ?? (() => {
        const tar = installedLinuxTar(opts.bundles);
        return fs.existsSync(tar) && fs.existsSync(`${tar}.sha256`) ? tar : null;
      }),
    },
  });
}

describe("versions", () => {
  it("compares and checks inclusive ranges", () => {
    assert.equal(compareVersions("0.2.10", "0.2.9"), 1);
    assert.equal(compareVersions("v1.0.0", "1.0"), 0);
    assert.equal(compareVersions("0.2.2-rc1", "0.2.2"), 0);
    assert.ok(versionInRange("0.2.2", { min: "0.2.2", max: "0.2.2" }));
    assert.ok(!versionInRange("0.2.3", { min: "0.2.2", max: "0.2.2" }));
    assert.ok(versionInRange("9.0.0", { min: "0.2.2" }));
    assert.ok(!versionInRange("0.2.1", { min: "0.2.2" }));
  });
});

describe("component source", () => {
  it("defaults to this version's GitHub Release", () => {
    const src = resolveComponentSource("0.2.2", {});
    assert.equal(src.kind, "github");
    assert.equal(src.kind === "github" && src.manifestUrl,
      "https://github.com/itissika/litecode/releases/download/v0.2.2/litecode-components.json");
  });

  it("honours LITECODE_COMPONENTS_BASE as URL or directory", () => {
    const url = resolveComponentSource("0.2.2", { LITECODE_COMPONENTS_BASE: "http://127.0.0.1:8080/c/" });
    assert.deepEqual(url, { kind: "url", base: "http://127.0.0.1:8080/c", manifestUrl: "http://127.0.0.1:8080/c/litecode-components.json" });
    const dir = resolveComponentSource("0.2.2", { LITECODE_COMPONENTS_BASE: "some/dir" });
    assert.deepEqual(dir, { kind: "dir", dir: path.resolve("some/dir") });
  });

  it("refuses non-GitHub-Release URLs from the default manifest", () => {
    const src = resolveComponentSource("1.2.3", {});
    const comp = {
      id: "embed-model", version: "x", platform: "any", file: "a.tar.gz", url: "https://evil.example/a.tar.gz",
      size: 1, sha256: "0".repeat(64), format: "tar.gz" as const, app_version_range: { min: "1.0.0" },
    };
    assert.throws(() => componentPayloadLocation(src, comp), /GitHub Releases/);
  });
});

describe("manifest", () => {
  it("rejects malformed entries", () => {
    assert.throws(() => parseManifest({ schema: 2, components: [] }), /schema/);
    assert.throws(() => parseManifest({ schema: 1, components: [{ id: "x" }] }), /sha256|version/);
    assert.throws(
      () => parseManifest({ schema: 1, components: [{ id: "x", version: "1", platform: "any", file: "../x", url: "u", size: 1, sha256: "a".repeat(64), format: "file", app_version_range: { min: "1" } }] }),
      /plain asset name/,
    );
  });

  it("is produced by build_components.mjs with GitHub Release URLs", () => {
    const { out, manifest } = buildComponents();
    assert.equal(manifest.tag, "v1.2.3");
    const ids = manifest.components.map((c) => c.id).sort();
    assert.deepEqual(ids, ["embed-model", "linux-server"]);
    for (const c of manifest.components) {
      assert.ok(c.url.startsWith("https://github.com/itissika/litecode/releases/download/v1.2.3/"), c.url);
      const bytes = fs.readFileSync(path.join(out, c.file));
      assert.equal(bytes.length, c.size);
      assert.equal(sha(bytes), c.sha256);
    }
    const linux = selectComponent(manifest, "linux-server", "1.2.3");
    assert.deepEqual(linux.app_version_range, { min: "1.2.3", max: "1.2.3" });
    assert.throws(() => selectComponent(manifest, "linux-server", "1.2.4"), /compatible/);
  });
});

/** Tiny ustar writer for hostile/edge-case archives. */
function tarGz(entries: Array<{ name: string; type?: string; body?: string }>): Buffer {
  const blocks: Buffer[] = [];
  for (const e of entries) {
    const body = Buffer.from(e.body ?? "");
    const h = Buffer.alloc(512, 0);
    h.write(e.name, 0, 100, "utf8");
    h.write("0000644\0", 100);
    h.write(`${body.length.toString(8).padStart(11, "0")}\0`, 124);
    h.fill(0x20, 148, 156);
    h.write(e.type ?? "0", 156);
    h.write("ustar\0", 257);
    let sum = 0;
    for (const b of h) sum += b;
    h.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148);
    blocks.push(h, body, Buffer.alloc((512 - (body.length % 512)) % 512, 0));
  }
  blocks.push(Buffer.alloc(1024, 0));
  return zlib.gzipSync(Buffer.concat(blocks));
}

describe("extractTarGz", () => {
  it("accepts a ./ root entry (bsdtar -C dir .) and nested files", async () => {
    const dir = tmp("lc-tar-");
    const archive = path.join(dir, "a.tar.gz");
    fs.writeFileSync(archive, tarGz([
      { name: "./", type: "5" },
      { name: "./sub/", type: "5" },
      { name: "./sub/x.txt", body: "hello" },
    ]));
    const written = await extractTarGz(archive, path.join(dir, "out"));
    assert.deepEqual(written, ["sub/x.txt"]);
    assert.equal(fs.readFileSync(path.join(dir, "out", "sub", "x.txt"), "utf8"), "hello");
  });

  it("rejects path traversal and links", async () => {
    const dir = tmp("lc-tar-bad-");
    for (const [i, entry] of [
      { name: "../evil.txt", body: "x" },
      { name: "/abs.txt", body: "x" },
      { name: "link", type: "2" },
    ].entries()) {
      const archive = path.join(dir, `${i}.tar.gz`);
      fs.writeFileSync(archive, tarGz([entry]));
      await assert.rejects(extractTarGz(archive, path.join(dir, `out${i}`)), /Unsafe path|Unsupported entry/);
    }
    assert.ok(!fs.existsSync(path.join(dir, "evil.txt")));
  });
});

describe("ComponentManager", () => {
  it("installs both components from a local LITECODE_COMPONENTS_BASE folder", async () => {
    const { out } = buildComponents();
    const bundles = tmp("lc-bundles-");
    const events: ComponentProgressEvent[] = [];
    const mgr = manager({ bundles, env: { LITECODE_COMPONENTS_BASE: out }, events });

    const model = await mgr.ensure("embed-model");
    assert.equal(model.downloaded, true);
    assert.equal(model.path, installedModelDir(bundles));
    assert.ok(isModelDirReady(model.path));
    assert.ok(model.path.endsWith(path.join("models", ...BUNDLED_MODEL_REL.split("/"))));
    assert.equal(fs.readFileSync(path.join(model.path, "tokenizer.json"), "utf8").length, 70_000);

    const linux = await mgr.ensure("linux-server");
    assert.equal(fs.readFileSync(linux.path, "utf8"), "linux-server-bytes");
    const pinned = fs.readFileSync(`${linux.path}.sha256`, "utf8").split(/\s+/)[0];
    assert.equal(pinned, sha(Buffer.from("linux-server-bytes")));

    assert.ok(events.some((e) => e.phase === "downloading"));
    assert.ok(events.some((e) => e.id === "embed-model" && e.phase === "done"));
    assert.equal(readInstallRecord(bundles).components["linux-server"]?.app_version, "1.2.3");
    assert.ok(!fs.readdirSync(path.join(bundles, ".tmp")).length, "temp files cleaned up");

    // Present → no second download.
    const again = await mgr.ensure("embed-model");
    assert.equal(again.downloaded, false);
  });

  it("downloads from GitHub via fetch, shares concurrent requests, and rejects bad hashes", async () => {
    const { out, manifest } = buildComponents();
    const calls: string[] = [];
    const serve = (corrupt: boolean): typeof fetch => (async (input: string | URL | Request) => {
      const url = String(input);
      calls.push(url);
      const name = url.split("/").pop()!;
      let body = fs.readFileSync(path.join(out, name));
      if (corrupt && name !== COMPONENTS_MANIFEST_NAME) body = Buffer.from(body.map((b, i) => (i === 0 ? b ^ 1 : b)));
      return new Response(body, { status: 200 });
    }) as typeof fetch;

    const badBundles = tmp("lc-bundles-bad-");
    const events: ComponentProgressEvent[] = [];
    const bad = manager({ bundles: badBundles, env: {}, fetchImpl: serve(true), events });
    await assert.rejects(bad.ensure("linux-server"), /sha256 mismatch/);
    assert.ok(!fs.existsSync(installedLinuxTar(badBundles)), "nothing installed on mismatch");
    assert.equal(bad.status("linux-server").error?.includes("sha256 mismatch"), true);
    assert.ok(events.some((e) => e.phase === "error"));

    calls.length = 0;
    const bundles = tmp("lc-bundles-gh-");
    const good = manager({ bundles, env: {}, fetchImpl: serve(false) });
    const [a, b] = await Promise.all([good.ensure("embed-model"), good.ensure("embed-model")]);
    assert.equal(a, b);
    const modelAsset = manifest.components.find((c) => c.id === "embed-model")!.file;
    assert.equal(calls.filter((u) => u.endsWith(modelAsset)).length, 1, "one shared download");
    assert.ok(calls[0]!.startsWith("https://github.com/itissika/litecode/releases/download/v1.2.3/"));
  });

  it("refreshes a managed Linux tar left over from another app version, keeps hand-placed ones", async () => {
    const { out } = buildComponents("1.2.4");
    const bundles = tmp("lc-bundles-stale-");
    const tar = installedLinuxTar(bundles);
    fs.mkdirSync(path.dirname(tar), { recursive: true });
    fs.writeFileSync(tar, "old");
    fs.writeFileSync(`${tar}.sha256`, `${"0".repeat(64)}  ${LINUX_TAR_NAME}\n`);

    // Hand-placed (no install record): trusted as before.
    const handPlaced = manager({ bundles, env: { LITECODE_COMPONENTS_BASE: out }, appVersion: "1.2.4" });
    assert.equal((await handPlaced.ensure("linux-server")).downloaded, false);

    // Recorded as downloaded for 1.2.3 → stale for 1.2.4 → re-downloaded.
    fs.writeFileSync(path.join(bundles, "components.json"), JSON.stringify({
      components: { "linux-server": { version: "1.2.3", sha256: "x", app_version: "1.2.3", installed_at: "" } },
    }));
    const upgraded = manager({ bundles, env: { LITECODE_COMPONENTS_BASE: out }, appVersion: "1.2.4" });
    const res = await upgraded.ensure("linux-server");
    assert.equal(res.downloaded, true);
    assert.equal(fs.readFileSync(tar, "utf8"), "linux-server-bytes");
  });

  it("reports a missing offline manifest as a readable, retryable error", async () => {
    const bundles = tmp("lc-bundles-missing-");
    const mgr = manager({ bundles, env: { LITECODE_COMPONENTS_BASE: path.join(bundles, "nope") } });
    await assert.rejects(mgr.ensure("embed-model"), /Cannot read .*litecode-components\.json[\s\S]*LITECODE_COMPONENTS_BASE/);
    assert.equal(mgr.status("embed-model").downloading, false);
  });

  it("clears the error state once the component is usable (retry, offline recovery, installed elsewhere)", async () => {
    const { out } = buildComponents();
    // Offline: every fetch throws. Error is reported, nothing installed.
    let online = false;
    const fetchImpl = (async (input: string | URL | Request) => {
      if (!online) throw new TypeError("fetch failed");
      return new Response(fs.readFileSync(path.join(out, String(input).split("/").pop()!)), { status: 200 });
    }) as typeof fetch;
    const bundles = tmp("lc-bundles-retry-");
    const events: ComponentProgressEvent[] = [];
    const mgr = manager({ bundles, env: {}, fetchImpl, events });
    await assert.rejects(mgr.ensure("embed-model"), /download failed/);
    let status = mgr.status("embed-model");
    assert.equal(status.installed, false);
    assert.ok(status.error, "offline failure is surfaced");
    assert.equal(events.at(-1)?.phase, "error");

    // Back online → Retry succeeds and the error is gone, with a final "done" event.
    online = true;
    assert.equal((await mgr.ensure("embed-model")).downloaded, true);
    status = mgr.status("embed-model");
    assert.deepEqual([status.installed, status.downloading, status.error], [true, false, null]);
    assert.equal(events.at(-1)?.phase, "done");

    // Failure recorded, then the model appears by other means (another window /
    // instance, hand copy): status must not stay red.
    const other = tmp("lc-bundles-elsewhere-");
    const offline = manager({ bundles: other, env: { LITECODE_COMPONENTS_BASE: path.join(other, "nope") } });
    await assert.rejects(offline.ensure("embed-model"));
    assert.ok(offline.status("embed-model").error);
    fs.cpSync(installedModelDir(bundles), installedModelDir(other), { recursive: true });
    assert.equal(offline.status("embed-model").error, null);
    assert.equal(offline.status("embed-model").installed, true);
    assert.equal((await offline.ensure("embed-model")).downloaded, false);
  });
});

describe("download UX helpers (D6/D7)", () => {
  it("tracks sliding-window speed and percent", () => {
    const speed = new SpeedTracker();
    speed.push(0, 1_000);
    speed.push(1_000_000, 2_000);
    assert.equal(speed.bytesPerSecond(), 1_000_000);
    assert.equal(progressPercent(50, 200), 25);
    assert.equal(progressPercent(undefined, 0), 0);
  });

  it("cleans stale .part and staging under bundles/.tmp", () => {
    const bundles = tmp("lc-tmp-clean-");
    const tmpDir = path.join(bundles, ".tmp");
    fs.mkdirSync(tmpDir, { recursive: true });
    fs.writeFileSync(path.join(tmpDir, "linux-server-abcdef.part"), "x");
    fs.mkdirSync(path.join(tmpDir, "embed-model-deadbeef"));
    fs.writeFileSync(path.join(tmpDir, "keep-me.txt"), "y");
    assert.equal(cleanupComponentTmp(tmpDir), 2);
    assert.deepEqual(fs.readdirSync(tmpDir).sort(), ["keep-me.txt"]);
  });
});

describe("ComponentManager abort + Range resume (D7)", () => {
  it("emits status, percent, and bytesPerSecond while downloading", async () => {
    const { out, manifest } = buildComponents();
    const linux = manifest.components.find((c) => c.id === "linux-server")!;
    const payload = fs.readFileSync(path.join(out, linux.file));

    let pullCount = 0;
    const fetchImpl = (async (input: string | URL | Request, _init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith(COMPONENTS_MANIFEST_NAME)) {
        return new Response(fs.readFileSync(path.join(out, COMPONENTS_MANIFEST_NAME)), { status: 200 });
      }
      // Stream in small chunks so progress fires with speed.
      pullCount += 1;
      const stream = new ReadableStream<Uint8Array>({
        start(controller) {
          const chunk = 4;
          let off = 0;
          const push = () => {
            if (off >= payload.length) {
              controller.close();
              return;
            }
            controller.enqueue(payload.subarray(off, off + chunk));
            off += chunk;
            setTimeout(push, 1);
          };
          push();
        },
      });
      return new Response(stream, { status: 200 });
    }) as typeof fetch;

    const bundles = tmp("lc-dl-speed-");
    const events: ComponentProgressEvent[] = [];
    const mgr = manager({ bundles, env: {}, fetchImpl, events });
    await mgr.ensure("linux-server");
    const downloading = events.filter((e) => e.phase === "downloading" && e.received && e.received > 0);
    assert.ok(downloading.length >= 1, "expected progress events");
    assert.ok(downloading.some((e) => typeof e.percent === "number"));
    assert.ok(downloading.some((e) => e.status === "downloading"));
    assert.ok(downloading.some((e) => typeof e.bytesPerSecond === "number"));
    assert.equal(pullCount >= 1, true);
  });

  it("abortAll keeps .part and the next ensure Range-resumes", async () => {
    const { out, manifest } = buildComponents();
    const linux = manifest.components.find((c) => c.id === "linux-server")!;
    const full = fs.readFileSync(path.join(out, linux.file));
    const mid = Math.floor(full.length / 2);

    let rangeSeen = false;
    let fullFetches = 0;
    let gate!: () => void;
    const blocked = new Promise<void>((r) => {
      gate = r;
    });
    let blockOnce = true;

    const fetchImpl = (async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith(COMPONENTS_MANIFEST_NAME)) {
        return new Response(fs.readFileSync(path.join(out, COMPONENTS_MANIFEST_NAME)), { status: 200 });
      }
      const headers = (init?.headers ?? {}) as Record<string, string>;
      const range = headers.Range || headers.range;
      if (range) {
        rangeSeen = true;
        assert.equal(range, `bytes=${mid}-`);
        return new Response(full.subarray(mid), {
          status: 206,
          headers: { "Content-Range": `bytes ${mid}-${full.length - 1}/${full.length}` },
        });
      }
      fullFetches += 1;
      if (blockOnce) {
        blockOnce = false;
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            controller.enqueue(full.subarray(0, mid));
            await blocked;
            // After pause the reader is cancelled; ignore further pushes.
            try {
              controller.close();
            } catch {
              /* ignore */
            }
          },
        });
        return new Response(stream, { status: 200 });
      }
      return new Response(full, { status: 200 });
    }) as typeof fetch;

    const bundles = tmp("lc-dl-pause-");
    const events: ComponentProgressEvent[] = [];
    const mgr = manager({ bundles, env: {}, fetchImpl, events });

    const first = mgr.ensure("linux-server");
    // Wait until half is on disk.
    const part = componentPartPath(path.join(bundles, ".tmp"), "linux-server", linux.sha256);
    for (let i = 0; i < 200; i++) {
      if (fs.existsSync(part) && fs.statSync(part).size >= mid) break;
      await new Promise((r) => setTimeout(r, 10));
    }
    assert.ok(fs.existsSync(part), "partial .part should exist before pause");
    mgr.abortAll();
    gate();
    await assert.rejects(first, (err: unknown) => isDownloadControlError(err) && err.kind === "pause");
    assert.ok(fs.existsSync(part), "abort must keep .part");
    assert.equal(mgr.status("linux-server").status, "paused");

    // Next ensure should Range-resume from mid.
    const result = await mgr.ensure("linux-server");
    assert.equal(result.downloaded, true);
    assert.ok(rangeSeen, "expected Range resume request");
    assert.equal(fs.readFileSync(result.path).equals(full), true);
    assert.ok(!fs.existsSync(part), "part removed after success");

    void fullFetches;
  });

  it("falls back to full re-download when server ignores Range", async () => {
    const { out, manifest } = buildComponents();
    const linux = manifest.components.find((c) => c.id === "linux-server")!;
    const full = fs.readFileSync(path.join(out, linux.file));
    const bundles = tmp("lc-dl-norange-");

    let sawRange = false;
    const fetchImpl = (async (input: string | URL | Request, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith(COMPONENTS_MANIFEST_NAME)) {
        return new Response(fs.readFileSync(path.join(out, COMPONENTS_MANIFEST_NAME)), { status: 200 });
      }
      const headers = (init?.headers ?? {}) as Record<string, string>;
      if (headers.Range || headers.range) sawRange = true;
      // Pretend we do not support Range — always 200 with full body.
      return new Response(full, { status: 200 });
    }) as typeof fetch;

    // Construct first so startup tmp cleanup runs, then seed a partial .part.
    const mgr = manager({ bundles, env: {}, fetchImpl });
    const part = componentPartPath(path.join(bundles, ".tmp"), "linux-server", linux.sha256);
    fs.mkdirSync(path.dirname(part), { recursive: true });
    fs.writeFileSync(part, full.subarray(0, 4));
    const res = await mgr.ensure("linux-server");
    assert.equal(res.downloaded, true);
    assert.ok(sawRange, "client should attempt Range when .part exists");
    assert.equal(fs.readFileSync(res.path).equals(full), true);
  });

  it("abortAll pauses in-flight downloads (before-quit path)", async () => {
    const { out, manifest } = buildComponents();
    const linux = manifest.components.find((c) => c.id === "linux-server")!;
    const full = fs.readFileSync(path.join(out, linux.file));
    let release!: () => void;
    const hold = new Promise<void>((r) => {
      release = r;
    });
    const fetchImpl = (async (input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith(COMPONENTS_MANIFEST_NAME)) {
        return new Response(fs.readFileSync(path.join(out, COMPONENTS_MANIFEST_NAME)), { status: 200 });
      }
      const stream = new ReadableStream<Uint8Array>({
        async start(controller) {
          controller.enqueue(full.subarray(0, 2));
          await hold;
          try {
            controller.close();
          } catch {
            /* ignore */
          }
        },
      });
      return new Response(stream, { status: 200 });
    }) as typeof fetch;
    const bundles = tmp("lc-dl-abortall-");
    const mgr = manager({ bundles, env: {}, fetchImpl });
    const job = mgr.ensure("linux-server");
    const part = componentPartPath(path.join(bundles, ".tmp"), "linux-server", linux.sha256);
    for (let i = 0; i < 200; i++) {
      if (fs.existsSync(part)) break;
      await new Promise((r) => setTimeout(r, 10));
    }
    mgr.abortAll();
    release();
    await assert.rejects(job, DownloadControlError);
    assert.ok(fs.existsSync(part), "abortAll behaves like pause (keeps .part)");
  });
});
