import { spawn, type ChildProcess } from "node:child_process";
import path from "node:path";

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
const MAX_CAPTURED_CHARS = 8_000;
const DRAIN_MS = 80;

export function formatSidecarBootFailure(opts: {
  bin: string;
  workspace: string;
  code: number | null;
  signal?: NodeJS.Signals | null;
  timedOut?: boolean;
  timeoutMs?: number;
  output: string;
}): string {
  const lines: string[] = [];
  if (opts.timedOut) {
    lines.push(`sidecar timed out waiting for LITECODE_READY (${opts.timeoutMs}ms)`);
  } else {
    const code = opts.code == null ? "null" : String(opts.code);
    const signal = opts.signal ? ` signal=${opts.signal}` : "";
    lines.push(`sidecar exited before READY (code=${code}${signal})`);
  }
  lines.push(`binary: ${opts.bin}`);
  lines.push(`workspace: ${opts.workspace}`);
  lines.push(`log file: ${path.join(opts.workspace, ".litecode", "logs", "litecode.log")}`);
  const output = opts.output.trim();
  if (output) {
    lines.push("sidecar output:");
    lines.push(output);
  } else {
    lines.push("sidecar produced no stdout/stderr before exit.");
  }
  return lines.join("\n");
}

function clipOutput(text: string): string {
  if (text.length <= MAX_CAPTURED_CHARS) return text;
  return `${text.slice(0, MAX_CAPTURED_CHARS)}\n… [truncated]`;
}

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
  const readyUrl = await waitForReady(child, timeoutMs, { bin, workspace });
  return { process: child, readyUrl, token: opts.token, workspace };
}

function waitForReady(
  child: ChildProcess,
  timeoutMs: number,
  ctx: { bin: string; workspace: string },
): Promise<string> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let stdoutBuf = "";
    let stderrBuf = "";
    const captured: string[] = [];

    const fail = (err: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      console.error(err.message);
      reject(err);
    };

    const bootError = (extra: {
      timedOut?: boolean;
      code?: number | null;
      signal?: NodeJS.Signals | null;
    }) =>
      new Error(
        formatSidecarBootFailure({
          bin: ctx.bin,
          workspace: ctx.workspace,
          code: extra.code ?? child.exitCode,
          signal: extra.signal ?? child.signalCode,
          timedOut: extra.timedOut,
          timeoutMs,
          output: clipOutput(captured.join("")),
        }),
      );

    const timer = setTimeout(() => {
      try {
        child.kill();
      } catch {
        /* ignore */
      }
      fail(bootError({ timedOut: true }));
    }, timeoutMs);

    const onChunk = (chunk: Buffer | string, stream: "stdout" | "stderr") => {
      const text = typeof chunk === "string" ? chunk : chunk.toString("utf8");
      captured.push(text);
      const carry = stream === "stdout" ? stdoutBuf + text : stderrBuf + text;
      const parts = carry.split(/\r?\n/);
      const rest = parts.pop() ?? "";
      if (stream === "stdout") stdoutBuf = rest;
      else stderrBuf = rest;
      for (const line of parts) {
        const m = READY_RE.exec(line.trim());
        if (!m || settled) continue;
        settled = true;
        clearTimeout(timer);
        const url = m[1].endsWith("/") ? m[1] : `${m[1]}/`;
        resolve(url);
        return;
      }
    };

    if (!child.stdout || !child.stderr) {
      fail(new Error("sidecar stdio pipes missing"));
      return;
    }

    child.stdout.on("data", (chunk) => onChunk(chunk, "stdout"));
    child.stderr.on("data", (chunk) => onChunk(chunk, "stderr"));

    child.on("error", (err) => {
      fail(new Error(`${err.message}\nbinary: ${ctx.bin}\nworkspace: ${ctx.workspace}`));
    });
    child.on("exit", (code, signal) => {
      if (settled) return;
      setTimeout(() => {
        fail(bootError({ code, signal }));
      }, DRAIN_MS);
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
