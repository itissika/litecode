import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import { normalizeWorkspace } from "./lap-path";

/**
 * dockview layout snapshots, one file per local workspace.
 *
 * A local workbench is served from `http://127.0.0.1:<ephemeral port>` (see
 * sidecar.ts), so the page's browser storage is keyed by a port that changes
 * every launch — a snapshot saved there is unreachable on the next boot. This
 * host owns a stable file instead, keyed by the same workspace path identity
 * `recents.ts` uses. Remote workbenches keep browser storage: their origin does
 * not change per launch and they are not this process's workspace.
 */

/** Guard for the renderer-supplied payload; a layout snapshot is far smaller. */
const MAX_LAYOUT_CHARS = 1_000_000;

export function layoutFilePath(baseDir: string, workspace: string): string {
  const key = crypto
    .createHash("sha1")
    .update(normalizeWorkspace(workspace))
    .digest("hex");
  return path.join(baseDir, "layouts", `${key}.json`);
}

/** Raw snapshot for `workspace`, or null when none was saved yet. */
export function readWorkspaceLayout(
  baseDir: string,
  workspace: string | null,
): string | null {
  if (!workspace) return null;
  try {
    const raw = fs.readFileSync(layoutFilePath(baseDir, workspace), "utf8");
    return raw.trim().length > 0 ? raw : null;
  } catch {
    return null;
  }
}

/** Store the snapshot verbatim. Never throws: a lost layout rebuilds defaults. */
export function writeWorkspaceLayout(
  baseDir: string,
  workspace: string | null,
  payload: unknown,
): void {
  if (!workspace) return;
  if (typeof payload !== "string") return;
  if (payload.length === 0 || payload.length > MAX_LAYOUT_CHARS) return;
  try {
    const file = layoutFilePath(baseDir, workspace);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const temporary = `${file}.${process.pid}.tmp`;
    fs.writeFileSync(temporary, payload, "utf8");
    fs.renameSync(temporary, file);
  } catch {
    /* ignore */
  }
}
