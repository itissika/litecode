import {
  type KeyboardEvent as ReactKeyboardEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import { revealInFileManager, type TreeEntry } from "../api/workspace";
import { revealEdgePanel } from "../dockview/workbench/edges";
import i18n from "../i18n";
import { useConnectionStore } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import { useExplorerStore } from "../stores/explorerStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTreeStore } from "../stores/treeStore";
import { useGitStore } from "../stores/gitStore";
import {
  gitChangedDirs,
  gitFileLetters,
  gitStatusColor,
} from "../lib/gitStatus";
import { useTerminalTabs } from "../lib/litecodeTerminal";
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
import {
  FileTreeContextMenu,
  type FileTreeMenuItem,
} from "./FileTreeContextMenu";
import { FileTreeGlobHits, FileTreeGlobInput } from "./FileTreeGlob";
import { TreeIndentGuides, treeRowPaddingLeft } from "./TreeIndentGuides";
import { fileNameFromPath } from "../utils/language";
import { getFileIcon, FolderIcon } from "../utils/fileIcon";
import { isSelfOrDescendant, parentPath } from "../utils/path";
import { useToastStore } from "../stores/toastStore";
import { LITECODE_PATHS_MIME, parsePathPayload } from "../lib/dropPayload";
import { flattenVisibleRows, visibleEntryPaths } from "../lib/fileTreeVisible";
import { whenCentreSettled } from "../lib/centreSettle";

export { LITECODE_PATHS_MIME };

/** Folder tint: amber lerped toward the normal folder text so a "has changes"
 *  folder reads as a hint, not a shout. Tweak the 60% to taste. */
const CHANGED_DIR_CLASS =
  "text-[color-mix(in_srgb,var(--_dk-amber-500)_60%,var(--_dk-text-secondary))]";

/** Matches `py-0.5` + `text-sm` + 16px icon row. */
const TREE_ROW_PX = 22;

function entryKind(
  children: Record<string, TreeEntry[] | undefined>,
  path: string,
): "file" | "dir" | null {
  if (!path) return "dir";
  const parent = parentPath(path);
  return (
    children[parent]?.find((e) => e.path === path)?.kind ??
    (children[path] ? "dir" : "file")
  );
}

function dropDir(
  path: string | null,
  children: Record<string, TreeEntry[] | undefined>,
): string {
  if (path === null) return "";
  return entryKind(children, path) === "dir" ? path : parentPath(path);
}

function readInternalPaths(dt: DataTransfer): string[] | null {
  const raw = dt.getData(LITECODE_PATHS_MIME) || dt.getData("text/plain");
  return parsePathPayload(raw);
}

function handleDropEvent(
  e: React.DragEvent,
  targetPath: string | null,
  childrenMap: Record<string, TreeEntry[] | undefined>,
): void {
  const target = dropDir(targetPath, childrenMap);
  const osFiles = [...e.dataTransfer.files].filter(
    (f) => f.size > 0 || Boolean(f.type),
  );
  const internal = readInternalPaths(e.dataTransfer);
  if (
    osFiles.length > 0 &&
    !e.dataTransfer.types.includes(LITECODE_PATHS_MIME)
  ) {
    if (osFiles.every((f) => f.size === 0 && !f.type)) {
      useToastStore
        .getState()
        .showToast(i18n.t("explorer.folderDropUnsupported"), "info");
      return;
    }
    void importOsFiles(target, osFiles);
    return;
  }
  if (internal && internal.length) {
    const copy = e.ctrlKey || e.metaKey;
    const blocked = internal.some((p) => isSelfOrDescendant(p, target));
    if (blocked && !copy) {
      useToastStore
        .getState()
        .showToast(i18n.t("explorer.moveIntoSelf"), "error");
      return;
    }
    void moveOrCopyEntries(internal, target, copy);
  }
}

async function beginCreate(
  parent: string,
  kind: "newFile" | "newFolder",
): Promise<void> {
  if (parent) await useTreeStore.getState().expandDir(parent);
  useExplorerStore.getState().setInline({ kind, parent });
}

function openIntegratedTerminal(path: string, isDir: boolean): void {
  if (!revealEdgePanel("terminal")) {
    useToastStore
      .getState()
      .showToast(i18n.t("explorer.terminalUnavailable"), "error");
    return;
  }
  const cwd = isDir ? path : parentPath(path);
  useTerminalTabs.getState().open(cwd || undefined);
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
        useToastStore.getState().showToast(i18n.t(err), "error");
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
      className="relative flex w-full items-center gap-1 px-2 py-0.5 text-sm"
      style={{ paddingLeft: `${treeRowPaddingLeft(depth)}px` }}
    >
      <TreeIndentGuides depth={depth} />
      <span className="w-4 shrink-0" />
      {kind === "newFolder" ? (
        <FolderIcon
          size={16}
          weight="regular"
          className="h-4 w-4 shrink-0 text-(--_dk-fg-muted)"
        />
      ) : (
        (() => {
          const Glyph = getFileIcon("untitled");
          return (
            <Glyph
              size={16}
              weight="regular"
              className="h-4 w-4 shrink-0 text-(--_dk-fg-muted)"
            />
          );
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
  gitLetters,
  changedDirs,
}: {
  entry: TreeEntry;
  depth: number;
  visible: string[];
  gitLetters: Map<string, string>;
  changedDirs: Set<string>;
}) {
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
  const gitLetter = isDir ? null : (gitLetters.get(entry.path) ?? null);
  const dirChanged = isDir && changedDirs.has(entry.path);
  const isActive = !isDir && activePath === entry.path;
  const isCut =
    clipboard?.mode === "cut" && clipboard.paths.includes(entry.path);
  const renaming = inline?.kind === "rename" && inline.path === entry.path;
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
        if (
          dropTarget === entry.path ||
          dropTarget === parentPath(entry.path)
        ) {
          /* keep until next over */
        }
      }}
      className={`relative flex w-full cursor-default items-center gap-1 truncate px-2 py-0.5 text-left text-sm transition-colors hover:bg-(--_dk-ix-bg-hover) ${highlight} ${
        isCut || busy ? "opacity-50" : ""
      } ${isDrop && isDir ? "outline outline-1 outline-(--_dk-accent-hover)" : ""} ${
        focusPath === entry.path
          ? "ring-1 ring-inset ring-(--_dk-line-visible)"
          : ""
      }`}
      style={{ paddingLeft: `${treeRowPaddingLeft(depth)}px` }}
      data-path={entry.path}
      title={entry.path}
    >
      <TreeIndentGuides depth={depth} />
      <span className="w-4 shrink-0 text-center text-[10px] text-(--_dk-text-disabled)">
        {isDir ? (expanded ? "▼" : "▶") : ""}
      </span>
      {isDir ? (
        <FolderIcon
          size={16}
          weight="regular"
          aria-hidden
          className={`h-4 w-4 shrink-0 select-none ${
            dirChanged ? CHANGED_DIR_CLASS : "text-(--_dk-fg-muted)"
          }`}
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
        <span className={`truncate ${dirChanged ? CHANGED_DIR_CLASS : ""}`}>
          {entry.name}
        </span>
      )}
      <span className="ml-auto flex shrink-0 items-center gap-1">
        {loading && (
          <span className="text-[10px] text-(--_dk-text-disabled)">…</span>
        )}
        {gitLetter && (
          <span
            className={`font-mono text-[11px] ${gitStatusColor(gitLetter)}`}
          >
            {gitLetter}
          </span>
        )}
      </span>
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
  const hostOs = useConnectionStore((s) => s.hostOs);
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
  const activePath = useEditorStore((s) => s.activePath);
  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [globQuery, setGlobQuery] = useState("");
  const filtering = globQuery.trim().length > 0;
  // Last path we scrolled into view. Guards the scroll effect so it only
  // scrolls when the active file *becomes* visible (tree re-renders on
  // expansion / refresh), never fighting the user's own browsing.
  const revealedRef = useRef<string | null>(null);

  const gitStatus = useGitStore((s) => s.status);
  const gitLetters = useMemo(() => gitFileLetters(gitStatus), [gitStatus]);
  const changedDirs = useMemo(() => gitChangedDirs(gitStatus), [gitStatus]);

  const ghost =
    inline && (inline.kind === "newFile" || inline.kind === "newFolder")
      ? { parent: inline.parent, kind: inline.kind }
      : null;
  const rows = useMemo(
    () => flattenVisibleRows(childrenMap, expanded, ghost),
    [childrenMap, expanded, ghost],
  );
  const visible = useMemo(() => visibleEntryPaths(rows), [rows]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => TREE_ROW_PX,
    overscan: 8,
    paddingEnd: 80,
  });

  // The root listing waits for the centre to settle (first chat, restored
  // list, or a failed socket) so it does not compete with session/new.
  useEffect(() => {
    let alive = true;
    void whenCentreSettled().then(() => {
      if (alive) void loadRoot();
    });
    return () => {
      alive = false;
    };
  }, [loadRoot]);

  // Follow the active editor file: expand its ancestor dirs, then scroll the
  // row into view once it is rendered. Runs whenever the active file changes
  // (tab switch, clicking into an editor panel, agent-opened files) — the
  // tree no longer needs the file pre-expanded to show it.
  useEffect(() => {
    if (!activePath) return;
    void useTreeStore.getState().revealPath(activePath);
  }, [activePath]);

  useEffect(() => {
    const p = activePath;
    if (!p || revealedRef.current === p) return;
    const idx = rows.findIndex((r) => r.type === "entry" && r.entry.path === p);
    if (idx < 0) return;
    revealedRef.current = p;
    virtualizer.scrollToIndex(idx, { align: "auto" });
  }, [activePath, rows, virtualizer]);

  // Seed status so change badges render before Source Control has loaded.
  // The commit log is the panel's own refresh.
  useEffect(() => {
    void useGitStore.getState().refresh({ silent: true, history: false });
  }, []);

  const operatePaths = (): string[] => {
    if (menu?.path && selected.has(menu.path)) return [...selected];
    if (menu?.path) return [menu.path];
    if (selected.size > 0) return [...selected];
    if (focusPath) return [focusPath];
    return [];
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement | null;
    if (target?.closest("input, textarea")) return;
    if (inline) {
      if (e.key === "Escape") setInline(null);
      return;
    }
    const mod = e.ctrlKey || e.metaKey;
    const paths =
      selected.size > 0 ? [...selected] : focusPath ? [focusPath] : [];
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
      if (filtering) {
        setGlobQuery("");
        return;
      }
      setMenu(null);
      clearSelection();
      return;
    }
    if (e.key === "Enter" && focus) {
      e.preventDefault();
      const kind = entryKind(childrenMap, focus);
      if (kind === "dir")
        void useTreeStore.getState().toggleExpand(focus, "dir");
      else void useEditorStore.getState().openFile(focus);
      return;
    }
    if (
      e.key === "ArrowDown" ||
      e.key === "ArrowUp" ||
      e.key === "Home" ||
      e.key === "End"
    ) {
      e.preventDefault();
      if (visible.length === 0) return;
      let idx = focus ? visible.indexOf(focus) : -1;
      if (e.key === "Home") idx = 0;
      else if (e.key === "End") idx = visible.length - 1;
      else if (e.key === "ArrowDown")
        idx = Math.min(visible.length - 1, idx + 1);
      else idx = Math.max(0, idx <= 0 ? 0 : idx - 1);
      const next = visible[idx];
      if (next) {
        select(next, { range: e.shiftKey, visible });
        setFocus(next);
        const rowIdx = rows.findIndex(
          (r) => r.type === "entry" && r.entry.path === next,
        );
        if (rowIdx >= 0) virtualizer.scrollToIndex(rowIdx, { align: "auto" });
      }
    }
  };

  const revealInHost = (target: string): void => {
    void revealInFileManager(target).catch((reason: unknown) => {
      useToastStore.getState().showToast(
        i18n.t("explorer.revealFailed", {
          message: reason instanceof Error ? reason.message : String(reason),
        }),
        "error",
      );
    });
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
        label: i18n.t("explorer.newFile"),
        onClick: () => void beginCreate(parent, "newFile"),
      },
      {
        id: "new-folder",
        label: i18n.t("explorer.newFolder"),
        onClick: () => void beginCreate(parent, "newFolder"),
      },
      { id: "sep-1", separator: true },
      ...(single && !isDir
        ? [
            {
              id: "open",
              label: i18n.t("explorer.open"),
              onClick: () => void useEditorStore.getState().openFile(single),
            } satisfies FileTreeMenuItem,
          ]
        : []),
      {
        id: "terminal",
        label: i18n.t("explorer.openInTerminal"),
        onClick: () =>
          openIntegratedTerminal(path ?? "", path === null ? true : isDir),
      },
      // Only a Windows serve host has a file manager to show things in; a
      // remote session's server must never pop its own.
      ...(path !== null && hostOs === "windows"
        ? [
            {
              id: "reveal",
              label: i18n.t("explorer.reveal"),
              onClick: () => revealInHost(path),
            } satisfies FileTreeMenuItem,
          ]
        : []),
      { id: "sep-2", separator: true },
      {
        id: "cut",
        label: i18n.t("explorer.cut"),
        shortcut: "Ctrl+X",
        disabled: paths.length === 0,
        onClick: () => setClipboard({ mode: "cut", paths }),
      },
      {
        id: "copy",
        label: i18n.t("explorer.copy"),
        shortcut: "Ctrl+C",
        disabled: paths.length === 0,
        onClick: () => setClipboard({ mode: "copy", paths }),
      },
      {
        id: "paste",
        label: i18n.t("explorer.paste"),
        shortcut: "Ctrl+V",
        disabled: !clipboard,
        onClick: () => void pasteEntries(parent),
      },
      {
        id: "dup",
        label: i18n.t("explorer.duplicate"),
        disabled: paths.length === 0,
        onClick: () => void duplicateEntries(paths),
      },
      {
        id: "rename",
        label: i18n.t("explorer.rename"),
        shortcut: "F2",
        disabled: !single || path === null,
        onClick: () => single && setInline({ kind: "rename", path: single }),
      },
      { id: "sep-3", separator: true },
      {
        id: "copy-path",
        label: i18n.t("explorer.copyPath"),
        onClick: () =>
          void copyAbsolutePaths(paths.length ? paths : [path ?? ""], project),
      },
      {
        id: "copy-rel",
        label: i18n.t("explorer.copyRelativePath"),
        onClick: () =>
          void copyRelativePaths(paths.length ? paths : [path ?? ""]),
      },
      ...(path === null
        ? [
            { id: "sep-collapse", separator: true } satisfies FileTreeMenuItem,
            {
              id: "collapse",
              label: i18n.t("explorer.collapseAll"),
              onClick: () => collapseAll(),
            } satisfies FileTreeMenuItem,
          ]
        : []),
      { id: "sep-4", separator: true },
      {
        id: "delete",
        label: i18n.t("explorer.delete"),
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
      role={filtering ? "search" : "tree"}
      data-drop-zone="tree"
      className="flex min-h-0 h-full flex-col outline-none"
      onKeyDown={onKeyDown}
      onClick={() => rootRef.current?.focus()}
    >
      <div className="flex shrink-0 flex-col gap-1 px-2 py-1">
        <span
          className="truncate px-1 text-[11px] text-(--_dk-text-disabled)"
          title={project || undefined}
        >
          {project
            ? fileNameFromPath(project) || project
            : i18n.t("explorer.project")}
        </span>
        <FileTreeGlobInput query={globQuery} onQueryChange={setGlobQuery} />
      </div>

      {filtering ? (
        <FileTreeGlobHits
          query={globQuery.trim()}
          onClear={() => setGlobQuery("")}
        />
      ) : (
        <div
          ref={scrollRef}
          className={`min-h-0 flex-1 overflow-y-auto py-1 ${
            dropTarget === ""
              ? "outline outline-1 outline-(--_dk-accent-hover)"
              : ""
          }`}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({ x: e.clientX, y: e.clientY, path: null });
          }}
          onDragOver={(e) => {
            e.preventDefault();
            e.dataTransfer.dropEffect =
              e.ctrlKey || e.metaKey ? "copy" : "move";
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
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              position: "relative",
              width: "100%",
            }}
          >
            {virtualizer.getVirtualItems().map((vi) => {
              const row = rows[vi.index];
              if (!row) return null;
              return (
                <div
                  key={
                    row.type === "entry"
                      ? row.entry.path
                      : `ghost:${row.parent}:${row.kind}`
                  }
                  data-index={vi.index}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    height: `${vi.size}px`,
                    transform: `translateY(${vi.start}px)`,
                  }}
                >
                  {row.type === "ghost" ? (
                    <GhostRow
                      parent={row.parent}
                      depth={row.depth}
                      kind={row.kind}
                    />
                  ) : (
                    <TreeNode
                      entry={row.entry}
                      depth={row.depth}
                      visible={visible}
                      gitLetters={gitLetters}
                      changedDirs={changedDirs}
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

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
