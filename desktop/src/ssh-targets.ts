import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

import { validateSshTarget, type SshTarget } from "./ssh-session";

export type SavedSshTarget = SshTarget & {
  id: string;
  label: string;
  /** ID of a safeStorage secret, never the secret itself. */
  credentialId?: string;
  credentialKind?: "private_key" | "password";
  /** Set only after a successful workspace attach. */
  lastWorkspace?: string;
  lastConnectedAt?: number;
  pinned?: boolean;
};

type TargetFile = {
  version: 1;
  targets: SavedSshTarget[];
};

export class SshTargets {
  constructor(private readonly filePath: string) {}

  list(): SavedSshTarget[] {
    return this.read().targets;
  }

  /** Remote history: only hosts that successfully attached at least once. */
  listConnected(): SavedSshTarget[] {
    return this.list()
      .filter((target) => typeof target.lastWorkspace === "string" && target.lastWorkspace.length > 0)
      .sort((a, b) => {
        if (Boolean(a.pinned) !== Boolean(b.pinned)) return a.pinned ? -1 : 1;
        return (b.lastConnectedAt ?? 0) - (a.lastConnectedAt ?? 0);
      });
  }

  save(input: Omit<SavedSshTarget, "id" | "lastConnectedAt"> & { id?: string }): SavedSshTarget {
    const target = validateSshTarget(input);
    const label = input.label.trim();
    if (!label) throw new Error("SSH target label is required.");
    const id = input.id?.trim() || crypto.randomUUID();
    const row: SavedSshTarget = {
      id,
      label,
      host: target.host,
      ...(target.user ? { user: target.user } : {}),
      ...(target.port ? { port: target.port } : {}),
      ...(target.identityFile ? { identityFile: target.identityFile } : {}),
      ...(input.credentialId ? { credentialId: input.credentialId } : {}),
      ...(input.credentialKind ? { credentialKind: input.credentialKind } : {}),
      ...(input.lastWorkspace ? { lastWorkspace: input.lastWorkspace } : {}),
      ...(typeof input.pinned === "boolean" ? { pinned: input.pinned } : {}),
      lastConnectedAt: Date.now(),
    };
    const file = this.read();
    const targets = [row, ...file.targets.filter((item) => item.id !== id)];
    this.write({ version: 1, targets });
    return row;
  }

  updateConnection(id: string, workspace: string): SavedSshTarget {
    const file = this.read();
    const existing = file.targets.find((target) => target.id === id);
    if (!existing) throw new Error("SSH target was not found.");
    return this.save({ ...existing, id, lastWorkspace: workspace });
  }

  setPinned(id: string, pinned: boolean): void {
    const file = this.read();
    const existing = file.targets.find((target) => target.id === id);
    if (!existing) return;
    this.save({ ...existing, id, pinned: Boolean(pinned) });
  }

  remove(id: string): SavedSshTarget | null {
    const file = this.read();
    const removed = file.targets.find((target) => target.id === id) ?? null;
    this.write({ version: 1, targets: file.targets.filter((target) => target.id !== id) });
    return removed;
  }

  get(id: string): SavedSshTarget {
    const target = this.read().targets.find((item) => item.id === id);
    if (!target) throw new Error("SSH target was not found.");
    return target;
  }

  private read(): TargetFile {
    try {
      const parsed: unknown = JSON.parse(fs.readFileSync(this.filePath, "utf8"));
      if (!parsed || typeof parsed !== "object" || (parsed as { version?: unknown }).version !== 1) {
        return { version: 1, targets: [] };
      }
      const targets = (parsed as { targets?: unknown }).targets;
      if (!Array.isArray(targets)) return { version: 1, targets: [] };
      return {
        version: 1,
        targets: targets.flatMap((item) => {
          if (!item || typeof item !== "object") return [];
          const row = item as Partial<SavedSshTarget>;
          if (typeof row.id !== "string" || typeof row.label !== "string") return [];
          try {
            const target = validateSshTarget(row as SshTarget);
            return [{
              id: row.id,
              label: row.label,
              host: target.host,
              ...(target.user ? { user: target.user } : {}),
              ...(target.port ? { port: target.port } : {}),
              ...(target.identityFile ? { identityFile: target.identityFile } : {}),
              ...(typeof row.credentialId === "string" ? { credentialId: row.credentialId } : {}),
              ...(row.credentialKind === "private_key" || row.credentialKind === "password" ? { credentialKind: row.credentialKind } : {}),
              ...(typeof row.lastWorkspace === "string" ? { lastWorkspace: row.lastWorkspace } : {}),
              ...(typeof row.lastConnectedAt === "number" ? { lastConnectedAt: row.lastConnectedAt } : {}),
              ...(typeof row.pinned === "boolean" ? { pinned: row.pinned } : {}),
            }];
          } catch {
            return [];
          }
        }),
      };
    } catch {
      return { version: 1, targets: [] };
    }
  }

  private write(file: TargetFile): void {
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true });
    const temporary = `${this.filePath}.${process.pid}.${Date.now()}.tmp`;
    fs.writeFileSync(temporary, `${JSON.stringify(file, null, 2)}\n`, "utf8");
    fs.renameSync(temporary, this.filePath);
  }
}
