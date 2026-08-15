/**
 * Desktop-side Litecode Absolute Path helpers (match Rust `config::path`).
 */
import fs from "node:fs";
import path from "node:path";

/** Match Rust `strip_verbatim` + drive uppercase (LAP shape). */
export function stripVerbatimLap(p: string): string {
  let s = p;
  if (s.startsWith("\\\\?\\UNC\\")) {
    s = `\\\\${s.slice("\\\\?\\UNC\\".length)}`;
  } else if (s.startsWith("\\\\?\\")) {
    s = s.slice("\\\\?\\".length);
  }
  if (/^[a-zA-Z]:/.test(s)) {
    s = s[0]!.toUpperCase() + s.slice(1);
  }
  return s;
}

/**
 * Align workspace paths with Rust LAP for multi-instance compare:
 * resolve → realpath (when exists) → strip verbatim → uppercase drive.
 */
export function normalizeWorkspace(ws: string): string {
  let resolved = path.resolve(ws);
  try {
    if (fs.existsSync(resolved)) {
      const real = fs.realpathSync.native?.(resolved) ?? fs.realpathSync(resolved);
      resolved = real;
    }
  } catch {
    /* keep resolved */
  }
  return stripVerbatimLap(resolved);
}
