import fs from "node:fs";
import path from "node:path";

import { normalizeWorkspace } from "./lap-path";

export type RecentWorkspace = {
  path: string;
  pinned: boolean;
  lastOpenedAt: number;
};

type RecentFile = {
  version: 1;
  workspaces: RecentWorkspace[];
};

const MAX_UNPINNED = 20;

function isRecentWorkspace(value: unknown): value is RecentWorkspace {
  if (!value || typeof value !== "object") return false;
  const row = value as Partial<RecentWorkspace>;
  return (
    typeof row.path === "string" &&
    row.path.trim().length > 0 &&
    typeof row.pinned === "boolean" &&
    typeof row.lastOpenedAt === "number" &&
    Number.isFinite(row.lastOpenedAt)
  );
}

/** Persistent, corruption-tolerant list of local workspaces. */
export class RecentWorkspaces {
  constructor(private readonly filePath: string) {}

  list(): RecentWorkspace[] {
    return this.read().workspaces;
  }

  record(workspacePath: string): RecentWorkspace[] {
    const target = normalizeWorkspace(workspacePath);
    const file = this.read();
    const existing = file.workspaces.find((row) => row.path === target);
    const next: RecentWorkspace = {
      path: target,
      pinned: existing?.pinned ?? false,
      lastOpenedAt: Date.now(),
    };
    this.write({
      version: 1,
      workspaces: this.sortAndLimit([
        next,
        ...file.workspaces.filter((row) => row.path !== target),
      ]),
    });
    return this.list();
  }

  setPinned(workspacePath: string, pinned: boolean): RecentWorkspace[] {
    const target = normalizeWorkspace(workspacePath);
    const file = this.read();
    const row = file.workspaces.find((item) => item.path === target);
    if (!row) return file.workspaces;
    row.pinned = Boolean(pinned);
    this.write({ version: 1, workspaces: this.sortAndLimit(file.workspaces) });
    return this.list();
  }

  remove(workspacePath: string): RecentWorkspace[] {
    const target = normalizeWorkspace(workspacePath);
    const file = this.read();
    this.write({
      version: 1,
      workspaces: file.workspaces.filter((row) => row.path !== target),
    });
    return this.list();
  }

  private read(): RecentFile {
    try {
      const raw: unknown = JSON.parse(fs.readFileSync(this.filePath, "utf8"));
      if (!raw || typeof raw !== "object" || (raw as { version?: unknown }).version !== 1) {
        return { version: 1, workspaces: [] };
      }
      const rows = (raw as { workspaces?: unknown }).workspaces;
      if (!Array.isArray(rows)) return { version: 1, workspaces: [] };
      const unique = new Map<string, RecentWorkspace>();
      for (const row of rows) {
        if (!isRecentWorkspace(row)) continue;
        const workspace = normalizeWorkspace(row.path);
        const previous = unique.get(workspace);
        if (!previous || row.lastOpenedAt > previous.lastOpenedAt) {
          unique.set(workspace, {
            path: workspace,
            pinned: row.pinned || previous?.pinned === true,
            lastOpenedAt: row.lastOpenedAt,
          });
        }
      }
      return { version: 1, workspaces: this.sortAndLimit([...unique.values()]) };
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
        this.backupCorruptFile();
      }
      return { version: 1, workspaces: [] };
    }
  }

  private write(file: RecentFile): void {
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
    const temporary = `${this.filePath}.${process.pid}.${Date.now()}.tmp`;
    fs.writeFileSync(temporary, `${JSON.stringify(file, null, 2)}\n`, "utf8");
    fs.renameSync(temporary, this.filePath);
  }

  private backupCorruptFile(): void {
    try {
      fs.renameSync(this.filePath, `${this.filePath}.corrupt-${Date.now()}`);
    } catch {
      // A malformed or inaccessible history must not prevent startup.
    }
  }

  private sortAndLimit(rows: RecentWorkspace[]): RecentWorkspace[] {
    const sorted = [...rows].sort(
      (a, b) => Number(b.pinned) - Number(a.pinned) || b.lastOpenedAt - a.lastOpenedAt,
    );
    const pinned = sorted.filter((row) => row.pinned);
    const unpinned = sorted.filter((row) => !row.pinned).slice(0, MAX_UNPINNED);
    return [...pinned, ...unpinned];
  }
}
