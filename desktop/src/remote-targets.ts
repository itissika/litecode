import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

export type SavedRemoteTarget = {
  id: string;
  label: string;
  baseUrl: string;
  /** Identifier of the token in secure-store; the token is never serialized here. */
  credentialId: string;
  pinned: boolean;
  lastConnectedAt: number;
};

type TargetFile = { version: 1; targets: SavedRemoteTarget[] };

function normalizeBaseUrl(value: string): string {
  const url = new URL(value.trim());
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) {
    throw new Error("Remote server URL must be an HTTP(S) origin without embedded credentials.");
  }
  url.pathname = "";
  url.search = "";
  url.hash = "";
  return url.toString().replace(/\/$/, "");
}

/** Persistent remote-server identities. Authentication data lives only in safeStorage. */
export class RemoteTargets {
  constructor(private readonly filePath: string) {}

  list(): SavedRemoteTarget[] {
    return this.read().targets;
  }

  record(baseUrl: string, credentialId: string, label?: string): SavedRemoteTarget {
    const normalized = normalizeBaseUrl(baseUrl);
    const file = this.read();
    const existing = file.targets.find((target) => target.baseUrl === normalized);
    const target: SavedRemoteTarget = {
      id: existing?.id ?? crypto.randomUUID(),
      label: label?.trim() || existing?.label || normalized,
      baseUrl: normalized,
      credentialId,
      pinned: existing?.pinned ?? false,
      lastConnectedAt: Date.now(),
    };
    this.write({ version: 1, targets: [target, ...file.targets.filter((item) => item.id !== target.id)] });
    return target;
  }

  setPinned(id: string, pinned: boolean): void {
    const file = this.read();
    const target = file.targets.find((item) => item.id === id);
    if (!target) return;
    target.pinned = Boolean(pinned);
    this.write(file);
  }

  remove(id: string): SavedRemoteTarget | null {
    const file = this.read();
    const removed = file.targets.find((item) => item.id === id) ?? null;
    this.write({ version: 1, targets: file.targets.filter((item) => item.id !== id) });
    return removed;
  }

  get(id: string): SavedRemoteTarget {
    const target = this.read().targets.find((item) => item.id === id);
    if (!target) throw new Error("Remote server target was not found.");
    return target;
  }

  private read(): TargetFile {
    try {
      const file = JSON.parse(fs.readFileSync(this.filePath, "utf8")) as Partial<TargetFile>;
      if (file.version !== 1 || !Array.isArray(file.targets)) return { version: 1, targets: [] };
      return {
        version: 1,
        targets: file.targets.flatMap((item) => {
          if (!item || typeof item !== "object") return [];
          const row = item as Partial<SavedRemoteTarget>;
          if (typeof row.id !== "string" || typeof row.label !== "string" || typeof row.credentialId !== "string" || typeof row.pinned !== "boolean" || typeof row.lastConnectedAt !== "number") return [];
          try {
            return [{ ...row, baseUrl: normalizeBaseUrl(String(row.baseUrl)) } as SavedRemoteTarget];
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
