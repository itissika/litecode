/** Normalize tool absolute paths to workspace-relative paths (LAP-aware). */

function hasVerbatimMarker(p: string): boolean {
  const raw = p.replace(/\\/g, "/");
  const slashQ = `/${"?"}/`;
  return (
    p.startsWith(["\\\\", "?", "\\"].join("")) ||
    p.startsWith(["\\\\", "?", "\\", "UNC\\"].join("")) ||
    raw.includes(slashQ) ||
    raw.startsWith(`//${"?"}/`)
  );
}

/** Slash unify + ASCII drive uppercase (align with litecodeLsp / Rust LAP). */
export function normalizeLapSlashes(p: string): string {
  let s = p.replace(/\\/g, "/").replace(/\/$/, "");
  s = s.replace(/^([a-z]):\//, (_, d: string) => `${d.toUpperCase()}:/`);
  return s;
}

/**
 * Strip `projectRoot` prefix from an absolute tool path.
 * Drive-letter compare is ASCII case-insensitive. Verbatim forms → null.
 */
export function toWorkspacePath(
  filePath: string,
  projectRoot?: string | null,
): string | null {
  if (!filePath) return null;
  if (hasVerbatimMarker(filePath) || (projectRoot && hasVerbatimMarker(projectRoot))) {
    return null;
  }

  const normalized = normalizeLapSlashes(filePath);

  if (projectRoot) {
    const root = normalizeLapSlashes(projectRoot);
    const pathCmp = normalized.toLowerCase();
    const rootCmp = root.toLowerCase();
    if (pathCmp === rootCmp) return "";
    if (pathCmp.startsWith(`${rootCmp}/`)) {
      return normalized.slice(root.length + 1);
    }
    // `/E:/proj/...` after URI-style strip
    const bare = normalized.replace(/^\//, "");
    const bareRoot = root.replace(/^\//, "");
    if (bare.toLowerCase().startsWith(`${bareRoot.toLowerCase()}/`)) {
      return bare.slice(bareRoot.length + 1);
    }
  }

  // Already workspace-relative (no leading slash / no drive).
  if (!normalized.startsWith("/") && !/^[a-zA-Z]:\//.test(normalized)) {
    return normalized;
  }

  return null;
}

/** Parent directory of a workspace-relative or absolute path (slash-normalized). */
export function parentPath(path: string): string {
  const n = path.replace(/\\/g, "/").replace(/\/$/, "");
  const idx = n.lastIndexOf("/");
  if (idx <= 0) return "";
  return n.slice(0, idx);
}

/** Join a workspace-relative parent with a child name. */
export function joinWorkspacePath(parent: string, name: string): string {
  const p = parent.replace(/\\/g, "/").replace(/\/$/, "");
  const n = name.replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
  if (!p) return n;
  if (!n) return p;
  return `${p}/${n}`;
}

/** Remap `path` when `from` was renamed/moved to `to` (including descendants). */
export function remapPathPrefix(path: string, from: string, to: string): string {
  if (path === from) return to;
  if (from && path.startsWith(`${from}/`)) {
    return `${to}${path.slice(from.length)}`;
  }
  return path;
}

export function isSelfOrDescendant(ancestor: string, candidate: string): boolean {
  if (!ancestor) return true;
  return candidate === ancestor || candidate.startsWith(`${ancestor}/`);
}
