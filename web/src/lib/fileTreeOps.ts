import {
  copyPath,
  createFile,
  deletePath,
  mkdir,
  renamePath,
  writeBlob,
} from "../api/workspace";
import { useEditorStore } from "../stores/editorStore";
import { useExplorerStore } from "../stores/explorerStore";
import { useToastStore } from "../stores/toastStore";
import { useTreeStore } from "../stores/treeStore";
import { fileNameFromPath } from "../utils/language";
import {
  isSelfOrDescendant,
  joinWorkspacePath,
  parentPath,
} from "../utils/path";
import { childNamesAt, uniqueChildName } from "./fileTreeNames";

function remapAfterMove(from: string, to: string): void {
  useEditorStore.getState().remapTabs(from, to);
  useExplorerStore.getState().remapPaths(from, to);
}

function toastError(err: unknown): void {
  let msg = err instanceof Error ? err.message : String(err);
  try {
    const parsed = JSON.parse(msg) as { error?: string };
    if (parsed.error) msg = parsed.error;
  } catch {
    /* keep raw */
  }
  useToastStore.getState().showToast(msg, "error");
}

function isConflict(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return /already exists|HTTP 409/i.test(msg);
}

async function refreshParents(paths: string[]): Promise<void> {
  const tree = useTreeStore.getState();
  const keys = new Set<string>();
  for (const p of paths) {
    keys.add(parentPath(p));
    keys.add(p);
  }
  keys.add("");
  for (const key of keys) {
    tree.invalidate(key);
  }
  const reloads = [...keys].filter(
    (key) => key === "" || tree.expanded.has(key),
  );
  await Promise.all(reloads.map((key) => useTreeStore.getState().loadChildren(key)));
}

async function withBusy<T>(paths: string[], fn: () => Promise<T>): Promise<T | undefined> {
  const explorer = useExplorerStore.getState();
  explorer.markBusy(paths);
  try {
    return await fn();
  } catch (err) {
    toastError(err);
    return undefined;
  } finally {
    useExplorerStore.getState().unmarkBusy(paths);
  }
}

export async function createNewFile(parent: string, name: string): Promise<string | undefined> {
  const dest = joinWorkspacePath(parent, name);
  return withBusy([dest], async () => {
    await createFile(dest, "");
    if (parent) await useTreeStore.getState().expandDir(parent);
    await refreshParents([dest]);
    await useEditorStore.getState().openFile(dest);
    return dest;
  });
}

export async function createNewFolder(parent: string, name: string): Promise<string | undefined> {
  const dest = joinWorkspacePath(parent, name);
  return withBusy([dest], async () => {
    await mkdir(dest);
    if (parent) await useTreeStore.getState().expandDir(parent);
    await refreshParents([dest]);
    await useTreeStore.getState().expandDir(dest);
    return dest;
  });
}

export async function renameEntry(from: string, toName: string): Promise<string | undefined> {
  const dest = joinWorkspacePath(parentPath(from), toName);
  if (dest === from) return from;
  return withBusy([from, dest], async () => {
    const result = await renamePath(from, dest, false);
    remapAfterMove(result.from, result.to);
    await refreshParents([result.from, result.to]);
    return result.to;
  });
}

export async function deleteEntries(paths: string[]): Promise<void> {
  const unique = [...new Set(paths)].filter(Boolean);
  if (unique.length === 0) return;
  const label =
    unique.length === 1
      ? unique[0]
      : `${unique.length} items`;
  const ok = window.confirm(
    unique.length === 1
      ? `Delete "${label}"?\n\nOn Windows this is sent to the Recycle Bin.`
      : `Delete ${label}?\n\nOn Windows this is sent to the Recycle Bin.`,
  );
  if (!ok) return;

  await withBusy(unique, async () => {
    for (const path of unique) {
      const tree = useTreeStore.getState();
      const isDir = tree.children[parentPath(path)]?.find((e) => e.path === path)?.kind === "dir"
        || Boolean(tree.children[path]);
      await deletePath(path, isDir);
      useEditorStore.getState().closeDeleted(path);
    }
    useExplorerStore.getState().clearSelection();
    await refreshParents(unique);
  });
}

export async function duplicateEntries(paths: string[]): Promise<void> {
  const tree = useTreeStore.getState();
  await withBusy(paths, async () => {
    for (const from of paths) {
      const parent = parentPath(from);
      const name = uniqueChildName(
        childNamesAt(tree.children, parent),
        fileNameFromPath(from),
      );
      const to = joinWorkspacePath(parent, name);
      await copyPath(from, to, false);
    }
    await refreshParents(paths);
  });
}

function destForPaste(from: string, targetDir: string, existing: string[]): string {
  const name = uniqueChildName(existing, fileNameFromPath(from));
  return joinWorkspacePath(targetDir, name);
}

export async function pasteEntries(targetDir: string): Promise<void> {
  const clip = useExplorerStore.getState().clipboard;
  if (!clip || clip.paths.length === 0) return;
  const tree = useTreeStore.getState();
  if (targetDir) await tree.expandDir(targetDir);

  const existing = () =>
    childNamesAt(useTreeStore.getState().children, targetDir);

  await withBusy([...clip.paths, targetDir], async () => {
    for (const from of clip.paths) {
      if (clip.mode === "cut" && isSelfOrDescendant(from, targetDir)) {
        useToastStore.getState().showToast("Cannot move a folder into itself", "error");
        continue;
      }
      const sameParent = parentPath(from) === targetDir;
      if (clip.mode === "cut" && sameParent) continue;

      if (clip.mode === "cut") {
        const exact = joinWorkspacePath(targetDir, fileNameFromPath(from));
        const collision = existing().some(
          (n) => n.toLowerCase() === fileNameFromPath(from).toLowerCase(),
        );
        const dest = collision ? destForPaste(from, targetDir, existing()) : exact;
        const result = await renamePath(from, dest, false);
        remapAfterMove(result.from, result.to);
      } else {
        let to = destForPaste(from, targetDir, existing());
        try {
          await copyPath(from, to, false);
        } catch (err) {
          if (isConflict(err)) {
            to = destForPaste(from, targetDir, [...existing(), fileNameFromPath(to)]);
            await copyPath(from, to, false);
          } else {
            throw err;
          }
        }
      }
    }
    if (clip.mode === "cut") {
      useExplorerStore.getState().setClipboard(null);
    }
    await refreshParents([...clip.paths, targetDir]);
  });
}

export async function moveOrCopyEntries(
  paths: string[],
  targetDir: string,
  copy: boolean,
): Promise<void> {
  const filtered = paths.filter((p) => p && !isSelfOrDescendant(p, targetDir));
  if (filtered.length === 0) return;
  if (targetDir) await useTreeStore.getState().expandDir(targetDir);

  await withBusy([...filtered, targetDir], async () => {
    for (const from of filtered) {
      if (parentPath(from) === targetDir && !copy) continue;
      const existing = childNamesAt(useTreeStore.getState().children, targetDir);
      const collision = existing.some(
        (n) => n.toLowerCase() === fileNameFromPath(from).toLowerCase(),
      );
      if (copy) {
        const to = uniqueChildName(existing, fileNameFromPath(from));
        await copyPath(from, joinWorkspacePath(targetDir, to), false);
      } else {
        const destName = collision
          ? uniqueChildName(existing, fileNameFromPath(from))
          : fileNameFromPath(from);
        const result = await renamePath(
          from,
          joinWorkspacePath(targetDir, destName),
          false,
        );
        remapAfterMove(result.from, result.to);
      }
    }
    if (!copy) useExplorerStore.getState().setClipboard(null);
    await refreshParents([...filtered, targetDir]);
  });
}

export async function importOsFiles(
  targetDir: string,
  files: File[],
): Promise<void> {
  if (files.length === 0) return;
  if (targetDir) await useTreeStore.getState().expandDir(targetDir);
  const max = 10 * 1024 * 1024;
  await withBusy([targetDir], async () => {
    for (const file of files) {
      if (file.size > max) {
        useToastStore
          .getState()
          .showToast(`"${file.name}" exceeds the 10 MB upload limit`, "error");
        continue;
      }
      const existing = childNamesAt(useTreeStore.getState().children, targetDir);
      const name = uniqueChildName(existing, file.name);
      const dest = joinWorkspacePath(targetDir, name);
      const bytes = new Uint8Array(await file.arrayBuffer());
      await writeBlob(dest, bytes, false);
    }
    await refreshParents([targetDir || "imported"]);
  });
}

export function copyRelativePaths(paths: string[]): Promise<void> {
  return navigator.clipboard.writeText(paths.join("\n"));
}

export function copyAbsolutePaths(paths: string[], projectRoot: string): Promise<void> {
  const isWin = /\\/.test(projectRoot) || /^[A-Za-z]:/.test(projectRoot);
  const sep = isWin ? "\\" : "/";
  const root = projectRoot.replace(/[\\/]+$/, "");
  const abs = paths.map((rel) => {
    if (!rel) return root;
    return `${root}${sep}${rel.replace(/\//g, sep)}`;
  });
  return navigator.clipboard.writeText(abs.join("\n"));
}
