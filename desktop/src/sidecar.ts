import { spawn, type ChildProcess } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";

import { resolveBundledModelDir } from "./bundle-paths";
import { normalizeWorkspace } from "./lap-path";
import { litecodeBinary, sidecarRoot } from "./paths";

export type SidecarHandle = {
  process: ChildProcess;
  readyUrl: string;
  token: string;
  workspace: string;
};

const READY_RE = /^LITECODE_READY (http:\/\/127\.0\.0\.1:\d+\/?)\s*$/;

export async function startSidecar(opts: {
  token: string;
  parentPid: number;
  /** Required: one sidecar process = one workspace. */
  workspace: string;
  timeoutMs?: number;
}): Promise<SidecarHandle> {
  const workspace = normalizeWorkspace(opts.workspace);
  const productRoot = sidecarRoot();
  const bin = litecodeBinary(productRoot);
  const args = [
    "--workspace",
    workspace,
    "serve",
    "--bind",
    "127.0.0.1:0",
    "--require-auth",
    "--parent-pid",
    String(opts.parentPid),
  ];

  const repoRoot = path.resolve(__dirname, "..", "..");
  const modelDir = resolveBundledModelDir({ sidecarRoot: productRoot, repoRoot });
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    LITECODE_TOKEN: opts.token,
  };
  if (modelDir && !process.env.LITECODE_MODEL_DIR?.trim()) {
    env.LITECODE_MODEL_DIR = modelDir;
  }

  const child = spawn(bin, args, {
    // Process cwd must equal workspace (same contract as Rust serve boot chdir).
    cwd: workspace,
    env,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });

  const timeoutMs = opts.timeoutMs ?? 90_000;
  const readyUrl = await waitForReady(child, timeoutMs);
  return { process: child, readyUrl, token: opts.token, workspace };
}

function waitForReady(child: ChildProcess, timeoutMs: number): Promise<string> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        try {
          child.kill();
        } catch {
          /* ignore */
        }
        reject(new Error(`timeout waiting for LITECODE_READY (${timeoutMs}ms)`));
      }
    }, timeoutMs);

    const onLine = (line: string) => {
      const m = READY_RE.exec(line.trim());
      if (!m || settled) return;
      settled = true;
      clearTimeout(timer);
      const url = m[1].endsWith("/") ? m[1] : `${m[1]}/`;
      resolve(url);
    };

    if (!child.stdout || !child.stderr) {
      clearTimeout(timer);
      reject(new Error("sidecar stdio pipes missing"));
      return;
    }

    const rlOut = createInterface({ input: child.stdout });
    const rlErr = createInterface({ input: child.stderr });
    rlOut.on("line", onLine);
    rlErr.on("line", onLine);

    child.on("error", (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(err);
    });
    child.on("exit", (code) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(new Error(`sidecar exited before READY (code=${code})`));
    });
  });
}

export async function stopSidecar(handle: SidecarHandle | null): Promise<void> {
  if (!handle) return;
  const child = handle.process;
  if (child.killed || child.exitCode !== null) return;

  await new Promise<void>((resolve) => {
    const done = () => resolve();
    child.once("exit", done);
    try {
      child.kill();
    } catch {
      resolve();
      return;
    }
    setTimeout(() => {
      if (child.exitCode === null) {
        try {
          child.kill("SIGKILL");
        } catch {
          /* ignore */
        }
        if (process.platform === "win32" && child.pid) {
          try {
            spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
              stdio: "ignore",
              windowsHide: true,
            });
          } catch {
            /* ignore */
          }
        }
      }
      resolve();
    }, 3000);
  });
}
