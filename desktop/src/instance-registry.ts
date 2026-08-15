import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { BrowserWindow } from "electron";

import { normalizeWorkspace } from "./lap-path";

export { normalizeWorkspace, stripVerbatimLap } from "./lap-path";

export type InstanceRecord = {
  pid: number;
  port: number;
  workspace: string | null;
  updatedAt: number;
};

function registryPath(): string {
  const base =
    process.env.LOCALAPPDATA ||
    process.env.XDG_DATA_HOME ||
    path.join(os.homedir(), ".local", "share");
  const dir = path.join(base, "litecode");
  fs.mkdirSync(dir, { recursive: true });
  return path.join(dir, "running-instances.json");
}

function readAll(): InstanceRecord[] {
  const p = registryPath();
  if (!fs.existsSync(p)) return [];
  try {
    const raw = JSON.parse(fs.readFileSync(p, "utf8")) as InstanceRecord[];
    return Array.isArray(raw) ? raw : [];
  } catch {
    return [];
  }
}

function writeAll(rows: InstanceRecord[]): void {
  fs.writeFileSync(registryPath(), JSON.stringify(rows, null, 2), "utf8");
}

function pidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function prune(rows: InstanceRecord[]): InstanceRecord[] {
  return rows.filter((r) => pidAlive(r.pid));
}

export class InstanceRegistry {
  private server: http.Server | null = null;
  private port = 0;
  private workspace: string | null = null;
  private mainWindow: BrowserWindow | null = null;

  async start(mainWindow: BrowserWindow): Promise<void> {
    this.mainWindow = mainWindow;
    this.server = http.createServer((req, res) => {
      if (req.method === "POST" && req.url === "/focus") {
        if (this.mainWindow && !this.mainWindow.isDestroyed()) {
          if (this.mainWindow.isMinimized()) this.mainWindow.restore();
          this.mainWindow.show();
          this.mainWindow.focus();
        }
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ ok: true }));
        return;
      }
      res.writeHead(404);
      res.end();
    });

    await new Promise<void>((resolve, reject) => {
      this.server!.once("error", reject);
      this.server!.listen(0, "127.0.0.1", () => {
        const addr = this.server!.address();
        if (addr && typeof addr === "object") {
          this.port = addr.port;
        }
        resolve();
      });
    });

    this.publish();
  }

  setWorkspace(workspace: string | null): void {
    this.workspace = workspace ? normalizeWorkspace(workspace) : null;
    this.publish();
  }

  private publish(): void {
    const rows = prune(readAll()).filter((r) => r.pid !== process.pid);
    rows.push({
      pid: process.pid,
      port: this.port,
      workspace: this.workspace,
      updatedAt: Date.now(),
    });
    writeAll(rows);
  }

  /** If another live instance owns `workspace`, ask it to focus and return true. */
  static async tryFocusWorkspace(workspace: string): Promise<boolean> {
    const target = normalizeWorkspace(workspace);
    const rows = prune(readAll());
    writeAll(rows);
    const hit = rows.find(
      (r) => r.workspace && normalizeWorkspace(r.workspace) === target && r.pid !== process.pid,
    );
    if (!hit) return false;
    try {
      await fetch(`http://127.0.0.1:${hit.port}/focus`, { method: "POST" });
      return true;
    } catch {
      return false;
    }
  }

  stop(): void {
    const rows = prune(readAll()).filter((r) => r.pid !== process.pid);
    writeAll(rows);
    if (this.server) {
      try {
        this.server.close();
      } catch {
        /* ignore */
      }
      this.server = null;
    }
  }
}
