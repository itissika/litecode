import type { GitFile, GitStatus } from "../api/workspace";
import { parentPath } from "../utils/path";

/**
 * Semantic change-type letter shared by the SCM panel and the file tree.
 * Staged vs unstaged is carried by the section, so "new" is always "N"
 * whether the file is untracked ("??") or a staged addition ("A").
 * "U" stays for conflicts.
 */
export function gitStatusLabel(file: GitFile): string {
  if (file.untracked || file.status === "A") return "N";
  return file.status || "M";
}

/** Color class for a status letter (shared with the SCM panel). */
export function gitStatusColor(letter: string): string {
  switch (letter) {
    case "N":
      return "text-(--_dk-emerald-500)"; // new
    case "D":
      return "text-(--_dk-red-500)"; // deleted
    case "U":
      return "text-(--_dk-cat-purple)"; // unmerged / conflict
    case "R":
      return "text-(--_dk-cat-blue)"; // renamed
    case "C":
      return "text-(--_dk-cat-cyan)"; // copied
    case "M":
    default:
      return "text-(--_dk-amber-500)"; // modified
  }
}

/** Change types the file tree surfaces: 增删改 (new / modified / deleted). */
const TREE_LETTERS = new Set(["N", "M", "D"]);

/** File-tree letter, or null when the change type isn't one of 增删改. */
export function treeGitLetter(file: GitFile): string | null {
  const letter = gitStatusLabel(file);
  return TREE_LETTERS.has(letter) ? letter : null;
}

const LETTER_PRIORITY: Record<string, number> = { N: 3, M: 2, D: 1 };

/** Per-file tree letter (staged + worktree merged, strongest wins). */
export function gitFileLetters(status: GitStatus): Map<string, string> {
  const out = new Map<string, string>();
  const consider = (file: GitFile) => {
    const letter = treeGitLetter(file);
    if (!letter) return;
    const prev = out.get(file.path);
    if (!prev || (LETTER_PRIORITY[letter] ?? 0) > (LETTER_PRIORITY[prev] ?? 0)) {
      out.set(file.path, letter);
    }
  };
  for (const f of status.staged) consider(f);
  for (const f of status.changes) consider(f);
  return out;
}

/** Directories whose subtree contains a tree-visible change. */
export function gitChangedDirs(status: GitStatus): Set<string> {
  const dirs = new Set<string>();
  for (const path of gitFileLetters(status).keys()) {
    let dir = parentPath(path);
    while (dir) {
      dirs.add(dir);
      dir = parentPath(dir);
    }
  }
  return dirs;
}
