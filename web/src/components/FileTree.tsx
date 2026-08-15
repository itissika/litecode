import { type KeyboardEvent as ReactKeyboardEvent, useEffect, useMemo, useRef } from "react";

import type { TreeEntry } from "../api/workspace";
import { getDockviewApi } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import { useExplorerStore } from "../stores/explorerStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTreeStore } from "../stores/treeStore";
import { openTerminalAt } from "../dockview/config/layout";
import {
  copyAbsolutePaths,
  copyRelativePaths,
  createNewFile,
  createNewFolder,
  deleteEntries,
  duplicateEntries,
  importOsFiles,
  moveOrCopyEntries,
  pasteEntries,
  renameEntry,
} from "../lib/fileTreeOps";
import { validateFileName } from "../lib/fileTreeNames";
import { FileTreeSkeleton } from "./ui/Skeleton";
import { FileTreeContextMenu, type FileTreeMenuItem } from "./FileTreeContextMenu";
import { fileNameFromPath } from "../utils/language";
import { getFileIcon, FolderIcon } from "../utils/fileIcon";
import { isSelfOrDescendant, parentPath } from "../utils/path";
import { useToastStore } from "../stores/toastStore";

export const LITECODE_PATHS_MIME = "application/x-litecode-paths";

function flattenVisible(
  children: Record<string, TreeEntry[] | undefined>,
  expanded: Set<string>,
  parent = "",
): TreeEntry[] {
  const list = children[parent] ?? [];
  const out: TreeEntry[] = [];
  for (const entry of list) {
    out.push(entry);
    if (entry.kind === "dir" && expanded.has(entry.path)) {
      out.push(...flattenVisible(children, expanded, entry.path));
    }
  }
  return out;
}

function entryKind(
  children: Record<string, TreeEntry[] | undefined>,
  path: string,
): "file" | "dir" | null {
  if (!path) return "dir";
  const parent = parentPath(path);
  return children[parent]?.find((e) => e.path === path)?.kind ?? (children[path] ? "dir" : "file");
}

function dropDir(path: string | null, children: Record<string, TreeEntry[] | undefined>): string {
  if (path === null) return "";
  return entryKind(children, path) === "dir" ? path : parentPath(path);
}

function readInternalPaths(dt: DataTransfer): string[] | null {
  const raw = dt.getData(LITECODE_PATHS_MIME) || dt.getData("text/plain");
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed) && parsed.every((p) => typeof p === "string")) {
      return parsed;
    }
  } catch {
    /* plain text paths */
  }
  return raw
    .split(/\r?\n/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function handleDropEvent(
  e: React.DragEvent,
  targetPath: string | null,
  childrenMap: Record<string, TreeEntry[] | undefined>,
): void {
  const target = dropDir(targetPath, childrenMap);
  const osFiles = [...e.dataTransfer.files].filter((f) => f.size > 0 || Boolean(f.type));
  const internal = readInternalPaths(e.dataTransfer);
  if (osFiles.length > 0 && !e.dataTransfer.types.includes(LITECODE_PATHS_MIME)) {
    if (osFiles.every((f) => f.size === 0 && !f.type)) {
      useToastStore.getState().showToast("Folder drops from the OS are not supported", "info");
      return;
    }
    void importOsFiles(target, osFiles);
    return;
  }
  if (internal && internal.length) {
    const copy = e.ctrlKey || e.metaKey;
    const blocked = internal.some((p) => isSelfOrDescendant(p, target));
    if (blocked && !copy) {
      useToastStore.getState().showToast("Cannot move a folder into itself", "error");
      return;
    }
    void moveOrCopyEntries(internal, target, copy);
  }
}

async function beginCreate(parent: string, kind: "newFile" | "newFolder"): Promise<void> {
  if (parent) await useTreeStore.getState().expandDir(parent);
  useExplorerStore.getState().setInline({ kind, parent });
}

function openIntegratedTerminal(path: string, isDir: boolean): void {
  const api = getDockviewApi();
  if (!api) {
    useToastStore.getState().showToast("Terminal is unavailable", "error");
    return;
  }
  const cwd = isDir ? path : parentPath(path);
  openTerminalAt(api, cwd);
}

function InlineNameInput({
  initial,
  onSubmit,
  onCancel,
}: {
  initial: string;
  onSubmit: (name: string) => void | Promise<void>;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);

  const finish = (submit: boolean) => {
    if (done.current) return;
    const value = (ref.current?.value ?? "").trim();
    if (submit) {
      const err = validateFileName(value);
      if (err) {
        useToastStore.getState().showToast(err, "error");
        ref.current?.focus();
        return;
      }
      done.current = true;
      void onSubmit(value);
      return;
    }
    done.current = true;
    onCancel();
  };

  return (
    <input
      ref={ref}
      defaultValue={initial}
      className="min-w-0 flex-1 rounded border border-(--_dk-accent-hover) bg-(--_dk-root) px-1 py-0 text-sm text-(--_dk-text-primary) outline-none"
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          finish(true);
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
    />
  );
}

function GhostRow({
  parent,
  depth,
  kind,
}: {
  parent: string;
  depth: number;
  kind: "newFile" | "newFolder";
}) {
  const setInline = useExplorerStore((s) => s.setInline);
  return (
    <div
      className="flex w-full items-center gap-1 px-2 py-0.5 text-sm"
      style={{ paddingLeft: `${depth * 12 + 8}px` }}
    >
      <span className="w-4 shrink-0" />
      {kind === "newFolder" ? (
        <FolderIcon size={16} weight="regular" className="h-4 w-4 shrink-0 text-(--_dk-fg-muted)" />
      ) : (
        (() => {
          const Glyph = getFileIcon("untitled");
          return <Glyph size={16} weight="regular" className="h-4 w-4 shrink-0 text-(--_dk-fg-muted)" />;
        })()
      )}
      <InlineNameInput
        initial={kind === "newFolder" ? "New Folder" : "untitled"}
        onCancel={() => setInline(null)}
        onSubmit={async (name) => {
          setInline(null);
          if (kind === "newFolder") await createNewFolder(parent, name);
          else await createNewFile(parent, name);
        }}
      />
    </div>
  );
}

function TreeNode({
  entry,
  depth,
  visible,
}: {
  entry: TreeEntry;
  depth: number;
  visible: string[];
}) {
  const children = useTreeStore((s) => s.children[entry.path]);
  const expanded = useTreeStore((s) => s.expanded.has(entry.path));
  const loading = useTreeStore((s) => s.loading.has(entry.path));
  const toggleExpand = useTreeStore((s) => s.toggleExpand);
  const openFile = useEditorStore((s) => s.openFile);
  const activePath = useEditorStore((s) => s.activePath);
  const selected = useExplorerStore((s) => s.selected.has(entry.path));
  const focusPath = useExplorerStore((s) => s.focusPath);
  const clipboard = useExplorerStore((s) => s.clipboard);
  const inline = useExplorerStore((s) => s.inline);
  const dropTarget = useExplorerStore((s) => s.dropTarget);
  const busy = useExplorerStore((s) => s.busy.has(entry.path));
  const select = useExplorerStore((s) => s.select);
  const setMenu = useExplorerStore((s) => s.setMenu);
  const setDropTarget = useExplorerStore((s) => s.setDropTarget);
  const setInline = useExplorerStore((s) => s.setInline);

  const isDir = entry.kind === "dir";
  const isActive = !isDir && activePath === entry.path;
  const isCut = clipboard?.mode === "cut" && clipboard.paths.includes(entry.path);
  const renaming = inline?.kind === "rename" && inline.path === entry.path;
  const ghostKind =
    isDir &&
    expanded &&
    inline &&
    (inline.kind === "newFile" || inline.kind === "newFolder") &&
    inline.parent === entry.path
      ? inline.kind
      : null;
  const isDrop = dropTarget === entry.path;

  const handleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    select(entry.path, {
      additive: e.ctrlKey || e.metaKey,
      range: e.shiftKey,
      visible,
    });
    if (e.ctrlKey || e.metaKey || e.shiftKey) return;
    if (isDir) void toggleExpand(entry.path, entry.kind);
    else void openFile(entry.path);
  };

  const onDragStart = (e: React.DragEvent) => {
    const sel = useExplorerStore.getState().selected;
    const paths = sel.has(entry.path) && sel.size > 0 ? [...sel] : [entry.path];
    e.dataTransfer.effectAllowed = "copyMove";
    e.dataTransfer.setData(LITECODE_PATHS_MIME, JSON.stringify(paths));
    e.dataTransfer.setData("text/plain", paths.join("\n"));
    e.dataTransfer.setData("text/uri-list", paths.join("\r\n"));
  };

  const onDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const target = isDir ? entry.path : parentPath(entry.path);
    const internal = e.dataTransfer.types.includes(LITECODE_PATHS_MIME);
    if (internal) {
      const copy = e.ctrlKey || e.metaKey;
      e.dataTransfer.dropEffect = copy ? "copy" : "move";
    } else {
      e.dataTransfer.dropEffect = "copy";
    }
    setDropTarget(target);
  };

  const highlight =
    selected || isActive
      ? "bg-(--_dk-ix-bg-selected) text-(--_dk-text-secondary)"
      : "text-(--_dk-text-secondary)";

  return (
    <div>
      <div
        role="treeitem"
        aria-selected={selected}
        draggable={!renaming}
        onClick={handleClick}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if (!useExplorerStore.getState().selected.has(entry.path)) {
            select(entry.path);
          }
          setMenu({ x: e.clientX, y: e.clientY, path: entry.path });
        }}
        onDragStart={onDragStart}
        onDragOver={onDragOver}
        onDrop={(e) => {
          e.preventDefault();
          e.stopPropagation();
          const target = isDir ? entry.path : parentPath(entry.path);
          handleDropEvent(e, target, useTreeStore.getState().children);
          useExplorerStore.getState().setDropTarget(null);
        }}
        onDragLeave={() => {
          if (dropTarget === entry.path || dropTarget === parentPath(entry.path)) {
            /* keep until next over */
          }
        }}
        className={`flex w-full cursor-default items-center gap-1 truncate px-2 py-0.5 text-left text-sm transition-colors hover:bg-(--_dk-ix-bg-hover) ${highlight} ${
          isCut || busy ? "opacity-50" : ""
        } ${isDrop && isDir ? "outline outline-1 outline-(--_dk-accent-hover)" : ""} ${
          focusPath === entry.path ? "ring-1 ring-inset ring-(--_dk-line-visible)" : ""
        }`}
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
        title={entry.path}
      >
        <span className="w-4 shrink-0 text-center text-[10px] text-(--_dk-text-disabled)">
          {isDir ? (expanded ? "▼" : "▶") : ""}
        </span>
        {isDir ? (
          <FolderIcon
            size={16}
            weight="regular"
            aria-hidden
            className="h-4 w-4 shrink-0 select-none text-(--_dk-fg-muted)"
          />
        ) : (
          (() => {
            const Glyph = getFileIcon(entry.name);
            return (
              <Glyph
                size={16}
                weight="regular"
                aria-hidden
                className="h-4 w-4 shrink-0 select-none text-(--_dk-fg-muted)"
              />
            );
          })()
        )}
        {renaming ? (
          <InlineNameInput
            initial={entry.name}
            onCancel={() => setInline(null)}
            onSubmit={async (name) => {
              setInline(null);
              await renameEntry(entry.path, name);
            }}
          />
        ) : (
          <span className="truncate">{entry.name}</span>
        )}
        {loading && (
          <span className="ml-auto text-[10px] text-(--_dk-text-disabled)">…</span>
        )}
      </div>
      {isDir && expanded && (
        <div>
          {ghostKind && (
            <GhostRow parent={entry.path} depth={depth + 1} kind={ghostKind} />
          )}
          {(children ?? []).map((child) => (
            <TreeNode
              key={child.path}
              entry={child}
              depth={depth + 1}
              visible={visible}
            />
          ))}
        </div>
      )}
    </div>
  );
}

export function FileTree() {
  const rootChildren = useTreeStore((s) => s.children[""]);
  const error = useTreeStore((s) => s.error);
  const loading = useTreeStore((s) => s.loading.has(""));
  const loadRoot = useTreeStore((s) => s.loadRoot);
  const collapseAll = useTreeStore((s) => s.collapseAll);
  const expanded = useTreeStore((s) => s.expanded);
  const childrenMap = useTreeStore((s) => s.children);
  const project = useSessionStore((s) => s.project);
  const menu = useExplorerStore((s) => s.menu);
  const inline = useExplorerStore((s) => s.inline);
  const clipboard = useExplorerStore((s) => s.clipboard);
  const selected = useExplorerStore((s) => s.selected);
  const focusPath = useExplorerStore((s) => s.focusPath);
  const setMenu = useExplorerStore((s) => s.setMenu);
  const setInline = useExplorerStore((s) => s.setInline);
  const setClipboard = useExplorerStore((s) => s.setClipboard);
  const setDropTarget = useExplorerStore((s) => s.setDropTarget);
  const dropTarget = useExplorerStore((s) => s.dropTarget);
  const select = useExplorerStore((s) => s.select);
  const setFocus = useExplorerStore((s) => s.setFocus);
  const clearSelection = useExplorerStore((s) => s.clearSelection);
  const rootRef = useRef<HTMLDivElement>(null);

  const visible = useMemo(
    () => flattenVisible(childrenMap, expanded).map((e) => e.path),
    [childrenMap, expanded],
  );

  useEffect(() => {
    void loadRoot();
  }, [loadRoot]);

  const rootGhostKind =
    inline &&
    (inline.kind === "newFile" || inline.kind === "newFolder") &&
    inline.parent === ""
      ? inline.kind
      : null;

  const operatePaths = (): string[] => {
    if (menu?.path && selected.has(menu.path)) return [...selected];
    if (menu?.path) return [menu.path];
    if (selected.size > 0) return [...selected];
    if (focusPath) return [focusPath];
    return [];
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (inline) {
      if (e.key === "Escape") setInline(null);
      return;
    }
    const mod = e.ctrlKey || e.metaKey;
    const paths = selected.size > 0 ? [...selected] : focusPath ? [focusPath] : [];
    const focus = focusPath ?? paths[0] ?? "";

    if (e.key === "F2" && paths[0]) {
      e.preventDefault();
      setInline({ kind: "rename", path: paths[0] });
      return;
    }
    if ((e.key === "Delete" || e.key === "Backspace") && paths.length) {
      e.preventDefault();
      void deleteEntries(paths);
      return;
    }
    if (mod && e.key.toLowerCase() === "c" && e.shiftKey) {
      e.preventDefault();
      void copyAbsolutePaths(paths.length ? paths : [""], project);
      return;
    }
    if (mod && e.altKey && e.key.toLowerCase() === "c") {
      e.preventDefault();
      void copyRelativePaths(paths.length ? paths : [""]);
      return;
    }
    if (mod && e.key.toLowerCase() === "c" && paths.length) {
      e.preventDefault();
      setClipboard({ mode: "copy", paths });
      return;
    }
    if (mod && e.key.toLowerCase() === "x" && paths.length) {
      e.preventDefault();
      setClipboard({ mode: "cut", paths });
      return;
    }
    if (mod && e.key.toLowerCase() === "v") {
      e.preventDefault();
      const dir = dropDir(focus || null, childrenMap);
      void pasteEntries(dir);
      return;
    }
    if (e.key === "Escape") {
      setMenu(null);
      clearSelection();
      return;
    }
    if (e.key === "Enter" && focus) {
      e.preventDefault();
      const kind = entryKind(childrenMap, focus);
      if (kind === "dir") void useTreeStore.getState().toggleExpand(focus, "dir");
      else void useEditorStore.getState().openFile(focus);
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      if (visible.length === 0) return;
      let idx = focus ? visible.indexOf(focus) : -1;
      if (e.key === "Home") idx = 0;
      else if (e.key === "End") idx = visible.length - 1;
      else if (e.key === "ArrowDown") idx = Math.min(visible.length - 1, idx + 1);
      else idx = Math.max(0, idx <= 0 ? 0 : idx - 1);
      const next = visible[idx];
      if (next) {
        select(next, { range: e.shiftKey, visible });
        setFocus(next);
      }
    }
  };

  const menuItems = (): FileTreeMenuItem[] => {
    const path = menu?.path ?? null;
    const paths = operatePaths();
    const kind = path === null ? "dir" : entryKind(childrenMap, path);
    const isDir = kind === "dir";
    const parent = path === null || isDir ? (path ?? "") : parentPath(path);
    const single = paths[0];
    return [
      {
        id: "new-file",
        label: "New File…",
        onClick: () => void beginCreate(parent, "newFile"),
      },
      {
        id: "new-folder",
        label: "New Folder…",
        onClick: () => void beginCreate(parent, "newFolder"),
      },
      { id: "sep-1", separator: true },
      ...(single && !isDir
        ? [
            {
              id: "open",
              label: "Open",
              onClick: () => void useEditorStore.getState().openFile(single),
            } satisfies FileTreeMenuItem,
          ]
        : []),
      {
        id: "terminal",
        label: "Open in Integrated Terminal",
        onClick: () =>
          openIntegratedTerminal(path ?? "", path === null ? true : isDir),
      },
      { id: "sep-2", separator: true },
      {
        id: "cut",
        label: "Cut",
        shortcut: "Ctrl+X",
        disabled: paths.length === 0,
        onClick: () => setClipboard({ mode: "cut", paths }),
      },
      {
        id: "copy",
        label: "Copy",
        shortcut: "Ctrl+C",
        disabled: paths.length === 0,
        onClick: () => setClipboard({ mode: "copy", paths }),
      },
      {
        id: "paste",
        label: "Paste",
        shortcut: "Ctrl+V",
        disabled: !clipboard,
        onClick: () => void pasteEntries(parent),
      },
      {
        id: "dup",
        label: "Duplicate",
        disabled: paths.length === 0,
        onClick: () => void duplicateEntries(paths),
      },
      {
        id: "rename",
        label: "Rename",
        shortcut: "F2",
        disabled: !single || path === null,
        onClick: () => single && setInline({ kind: "rename", path: single }),
      },
      { id: "sep-3", separator: true },
      {
        id: "copy-path",
        label: "Copy Path",
        onClick: () => void copyAbsolutePaths(paths.length ? paths : [path ?? ""], project),
      },
      {
        id: "copy-rel",
        label: "Copy Relative Path",
        onClick: () => void copyRelativePaths(paths.length ? paths : [path ?? ""]),
      },
      ...(path === null
        ? [
            { id: "sep-collapse", separator: true } satisfies FileTreeMenuItem,
            {
              id: "collapse",
              label: "Collapse All",
              onClick: () => collapseAll(),
            } satisfies FileTreeMenuItem,
          ]
        : []),
      { id: "sep-4", separator: true },
      {
        id: "delete",
        label: "Delete",
        shortcut: "Del",
        danger: true,
        disabled: paths.length === 0,
        onClick: () => void deleteEntries(paths),
      },
    ];
  };

  return (
    <div
      ref={rootRef}
      tabIndex={0}
      role="tree"
      className="flex min-h-0 h-full flex-col outline-none"
      onKeyDown={onKeyDown}
      onClick={() => rootRef.current?.focus()}
    >
      <div className="flex items-center px-2 py-1">
        <span
          className="truncate px-1 text-[11px] text-(--_dk-text-disabled)"
          title={project || undefined}
        >
          {project ? fileNameFromPath(project) || project : "Project"}
        </span>
      </div>

      <div
        className={`min-h-0 flex-1 overflow-y-auto py-1 ${
          dropTarget === "" ? "outline outline-1 outline-(--_dk-accent-hover)" : ""
        }`}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY, path: null });
        }}
        onDragOver={(e) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = e.ctrlKey || e.metaKey ? "copy" : "move";
          setDropTarget("");
        }}
        onDragLeave={() => setDropTarget(null)}
        onDrop={(e) => {
          e.preventDefault();
          handleDropEvent(e, dropTarget, childrenMap);
          setDropTarget(null);
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget) clearSelection();
        }}
      >
        {error && (
          <p className="px-3 py-2 text-xs text-(--_dk-red-500)">{error}</p>
        )}
        {loading && !rootChildren && <FileTreeSkeleton />}
        {rootGhostKind && <GhostRow parent="" depth={0} kind={rootGhostKind} />}
        {rootChildren?.map((entry) => (
          <TreeNode key={entry.path} entry={entry} depth={0} visible={visible} />
        ))}
        <div aria-hidden className="shrink-0" style={{ height: "30%" }} />
      </div>

      {menu && (
        <FileTreeContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems()}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
}
