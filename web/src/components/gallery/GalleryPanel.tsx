import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import type { IDockviewPanelProps } from "dockview-react";
import {
  ArrowClockwise,
  CaretRight,
  FolderSimple,
  Images,
  ListBullets,
  MagnifyingGlass,
  SortAscending,
  SortDescending,
  SquaresFour,
  X,
} from "@phosphor-icons/react";

import {
  fetchMedia,
  openWithDefaultApp,
  revealInFileManager,
  type MediaEntry,
  type MediaKind,
  type MediaListing,
} from "../../api/workspace";
import { formatBytes } from "../../lib/bytes";
import { fileKindFromPath, type FileKind } from "../../lib/fileKind";
import { useConnectionStore } from "../../stores/connectionStore";
import { useEditorStore } from "../../stores/editorStore";
import { useToastStore } from "../../stores/toastStore";
import { useWorkspaceChangeStore } from "../../stores/workspaceChangeStore";
import i18n from "../../i18n";
import { deleteEntries, renameEntry } from "../../lib/fileTreeOps";
import { validateFileName } from "../../lib/fileTreeNames";
import { applyListSelection } from "../../lib/listSelection";
import { ContextMenu, type ContextMenuItem } from "../ui/ContextMenu";
import { useConfirmDialog } from "../ui/useConfirmDialog";
import { GalleryGrid } from "./GalleryGrid";
import {
  MEDIA_KIND_ORDER,
  allSection,
  buildFolderIndex,
  capSections,
  childFolders,
  folderEntries,
  folderLabel,
  folderTrail,
  galleryComparator,
  kindSections,
  sectionsHaveMore,
  sortEntries,
  type GalleryMode,
  type GallerySection,
  type GallerySort,
  type GallerySortDirection,
  type GalleryView,
} from "./galleryGroups";

const MODES: readonly GalleryMode[] = ["all", "folder", "kind"];
const MODE_LABEL: Record<GalleryMode, string> = {
  all: "gallery.mode.all",
  folder: "gallery.mode.folder",
  kind: "gallery.mode.kind",
};

/** Kinds the editor panel renders itself; everything else goes to the OS. */
const PANEL_KINDS: ReadonlySet<FileKind> = new Set<FileKind>([
  "image",
  "pdf",
  "audio",
  "video",
  "sqlite",
]);
const PAGE = 120;
const EMPTY_ENTRIES: MediaEntry[] = [];
/** Namespaced like the other panel preferences, so a reload keeps the style. */
const VIEW_KEY = "litecode.gallery.view";

function loadView(): GalleryView {
  try {
    return localStorage.getItem(VIEW_KEY) === "list" ? "list" : "grid";
  } catch {
    return "grid";
  }
}
/** Coalesce a burst of workspace/changed frames into one listing request. */
const CHANGE_DEBOUNCE_MS = 400;

/** One listing request: n restarts the effect, fresh forces a real walk. */
interface ListingRequest {
  n: number;
  fresh: boolean;
}

/** The entry a rename produces: same file, new path and caption. */
function renamedEntry(entry: MediaEntry, next: string): MediaEntry {
  const slash = next.lastIndexOf("/");
  return {
    ...entry,
    path: next,
    name: slash >= 0 ? next.slice(slash + 1) : next,
    folder: slash >= 0 ? next.slice(0, slash) : "",
  };
}

function kindLabel(kind: MediaKind): string {
  return i18n.t(`gallery.kind.${kind}`);
}

/**
 * Workspace media browser. One server listing feeds three groupings: a flat
 * grid, a folder drill-down, and image / video / audio sections. Every grouping
 * honours one preview style — thumbnails or list — so folders and files always
 * share the same shape. The listing is a generation-keyed snapshot on the
 * server, so reopening the panel does not walk the tree; the panel re-reads
 * after a workspace change or a resumed connection. Thumbnails are fetched
 * lazily and cached by path + mtime; double click hands the original to the
 * system player.
 */
export function GalleryPanel(_props: IDockviewPanelProps) {
  const { confirm, dialog } = useConfirmDialog();
  const confirmDelete = useCallback(
    (message: string) =>
      confirm({
        title: i18n.t("common.delete"),
        body: message,
        danger: true,
        confirmLabel: i18n.t("common.delete"),
        cancelLabel: i18n.t("common.cancel"),
      }),
    [confirm],
  );
  const [listing, setListing] = useState<MediaListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [request, setRequest] = useState<ListingRequest>({ n: 0, fresh: false });

  const [mode, setMode] = useState<GalleryMode>("all");
  const [view, setView] = useState<GalleryView>(loadView);
  const [folder, setFolder] = useState("");
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<GallerySort>("name");
  const [sortDir, setSortDir] = useState<GallerySortDirection>("asc");
  const [selected, setSelected] = useState<ReadonlySet<string>>(
    () => new Set<string>(),
  );
  const [anchor, setAnchor] = useState<string | null>(null);
  const [active, setActive] = useState<string | null>(null);
  const [limit, setLimit] = useState(PAGE);
  // Types view opens as an overview: every kind folded until it is asked for.
  const [foldedKinds, setFoldedKinds] = useState<ReadonlySet<string>>(
    () => new Set<string>(MEDIA_KIND_ORDER),
  );
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    entry: MediaEntry;
  } | null>(null);
  // Path of the tile being renamed in place; the caption becomes an input.
  const [renamingPath, setRenamingPath] = useState<string | null>(null);

  const scrollRef = useRef<HTMLDivElement | null>(null);
  const sentinelRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    try {
      localStorage.setItem(VIEW_KEY, view);
    } catch {
      // Storage can be unavailable; the choice still holds for this session.
    }
  }, [view]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError(null);
    void fetchMedia({ fresh: request.fresh })
      .then((next) => {
        if (!alive) return;
        setListing(next);
        setLoading(false);
      })
      .catch((reason: unknown) => {
        if (!alive) return;
        setError(reason instanceof Error ? reason.message : String(reason));
        setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [request]);

  // Any workspace change may add or drop media. The burst is coalesced here and
  // the server answers from its snapshot, so one request per burst is enough.
  const workspaceChange = useWorkspaceChangeStore((state) => state.last);
  useEffect(() => {
    if (!workspaceChange) return;
    const timer = setTimeout(() => {
      setRequest((value) => ({ n: value.n + 1, fresh: false }));
    }, CHANGE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [workspaceChange]);

  // A resumed connection may have missed frames, or switched workspace, so it
  // asks for a walk instead of trusting the snapshot.
  const connection = useConnectionStore((state) => state.state);
  // Revealing exists only where the serve host has a file manager to pop.
  const hostOs = useConnectionStore((state) => state.hostOs);
  const wasConnected = useRef(connection === "connected");
  useEffect(() => {
    if (connection === "connected" && !wasConnected.current) {
      setRequest((value) => ({ n: value.n + 1, fresh: true }));
    }
    wasConnected.current = connection === "connected";
  }, [connection]);

  const entries = listing?.entries ?? EMPTY_ENTRIES;
  const entrySet = useMemo(
    () => new Set(entries.map((entry) => entry.path)),
    [entries],
  );
  const filteredEntries = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return entries;
    return entries.filter(
      (entry) =>
        entry.name.toLowerCase().includes(needle) ||
        entry.path.toLowerCase().includes(needle),
    );
  }, [entries, query]);
  const compare = useMemo(
    () => galleryComparator(sort, sortDir),
    [sort, sortDir],
  );
  const sortedEntries = useMemo(
    () => sortEntries(filteredEntries, sort, sortDir),
    [filteredEntries, sort, sortDir],
  );
  // Folder counts follow the current search, but folder validity follows the
  // workspace. A query must not reset the folder the user is standing in.
  const allFolderIndex = useMemo(() => buildFolderIndex(entries), [entries]);
  const folderIndex = useMemo(
    () => buildFolderIndex(filteredEntries),
    [filteredEntries],
  );
  const folders = useMemo(
    () => childFolders(folderIndex, folder),
    [folderIndex, folder],
  );

  const sections = useMemo(() => {
    if (mode === "all") return allSection(sortedEntries, compare);
    if (mode === "kind") return kindSections(sortedEntries, kindLabel, compare);
    const direct = folderEntries(sortedEntries, folder, compare);
    return direct.length ? [{ key: "folder", entries: direct, total: direct.length }] : [];
  }, [compare, folder, mode, sortedEntries]);

  const isFolded = useCallback(
    (section: GallerySection) =>
      Boolean(section.label) && foldedKinds.has(section.key),
    [foldedKinds],
  );
  const openSections = useMemo(
    () => sections.filter((section) => !isFolded(section)),
    [isFolded, sections],
  );
  // Folded sections stay in the list for their header, but carry no tiles: the
  // grid mounts nothing for them, so no thumbnail is fetched.
  const visibleSections = useMemo(() => {
    const capped = new Map(
      capSections(openSections, limit).map((section) => [section.key, section]),
    );
    return sections.map(
      (section) =>
        capped.get(section.key) ?? { ...section, entries: EMPTY_ENTRIES },
    );
  }, [limit, openSections, sections]);
  // Paging follows the sections that are actually open.
  const hasMore = useMemo(
    () => sectionsHaveMore(openSections, limit),
    [limit, openSections],
  );

  const selectedEntries = useMemo(
    () => entries.filter((entry) => selected.has(entry.path)),
    [entries, selected],
  );
  const activeEntry = useMemo(() => {
    const focused = entries.find((entry) => entry.path === active);
    if (focused && selected.has(focused.path)) return focused;
    return selectedEntries[0] ?? null;
  }, [active, entries, selected, selectedEntries]);
  const selectableEntries = useMemo(
    () => visibleSections.flatMap((section) => section.entries),
    [visibleSections],
  );

  // A reload may drop the drilled folder or the selected file. Keep a stale
  // selection from operating on paths that no longer exist.
  useEffect(() => {
    if (folder && !allFolderIndex.has(folder)) setFolder("");
  }, [allFolderIndex, folder]);
  useEffect(() => {
    setSelected((current) => {
      let changed = false;
      const next = new Set<string>();
      for (const path of current) {
        if (entrySet.has(path)) next.add(path);
        else changed = true;
      }
      return changed ? next : current;
    });
    setAnchor((current) =>
      current && entrySet.has(current) ? current : null,
    );
    setActive((current) =>
      current && entrySet.has(current) ? current : null,
    );
  }, [entrySet]);

  // Searching is a new scope; don't leave bulk actions pointing at rows the
  // user can no longer see.
  useEffect(() => {
    setSelected(new Set<string>());
    setAnchor(null);
  }, [query]);

  // Roving focus: exactly one tile is tabbable, and it follows the visible list.
  useEffect(() => {
    if (selectableEntries.length === 0) {
      if (active) setActive(null);
      return;
    }
    if (!active || !selectableEntries.some((entry) => entry.path === active)) {
      setActive(selectableEntries[0].path);
    }
  }, [active, selectableEntries]);

  // New scope: restart paging at the top.
  useEffect(() => {
    setLimit(PAGE);
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [mode, folder]);

  // Page in more tiles when the sentinel reaches the viewport. Re-observing on
  // every page also handles a sentinel that stays visible (tall panel).
  useEffect(() => {
    if (!hasMore) return;
    const node = sentinelRef.current;
    if (!node) return;
    if (typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (records) => {
        if (records.some((record) => record.isIntersecting)) {
          setLimit((value) => value + PAGE);
        }
      },
      { root: scrollRef.current, rootMargin: "320px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [hasMore, limit, mode, folder, foldedKinds]);

  // One open rule for the whole panel, matching the editor: a kind the panel
  // can render opens in its own tab, and a container it cannot decode (mkv,
  // aiff, heic) goes to the system player.
  const openEntry = useCallback((entry: MediaEntry) => {
    if (PANEL_KINDS.has(fileKindFromPath(entry.path))) {
      void useEditorStore.getState().openFile(entry.path);
      return;
    }
    void openWithDefaultApp(entry.path).catch((reason: unknown) => {
      useToastStore.getState().showToast(
        i18n.t("gallery.openFailed", {
          message: reason instanceof Error ? reason.message : String(reason),
        }),
        "error",
      );
    });
  }, []);

  const enterFolder = useCallback((path: string) => {
    setFolder(path);
    setSelected(new Set<string>());
    setAnchor(null);
    setActive(null);
  }, []);

  const clearSelection = useCallback(() => {
    setSelected(new Set<string>());
    setAnchor(null);
  }, []);

  const selectEntry = useCallback(
    (entry: MediaEntry, opts?: { additive?: boolean; range?: boolean }) => {
      const result = applyListSelection(selected, entry.path, anchor, {
        ...opts,
        visible: selectableEntries.map((item) => item.path),
      });
      setSelected(result.selected);
      setAnchor(result.anchor);
      setActive(entry.path);
    },
    [anchor, selectableEntries, selected],
  );

  const activateEntry = useCallback((entry: MediaEntry) => {
    setActive(entry.path);
  }, []);

  const focusEntry = useCallback((path: string) => {
    const root = scrollRef.current;
    if (!root) return;
    const node = Array.from(
      root.querySelectorAll<HTMLElement>("[data-gallery-path]"),
    ).find((candidate) => candidate.dataset.galleryPath === path);
    if (!node) return;
    node.focus({ preventScroll: true });
    node.scrollIntoView?.({ block: "nearest" });
  }, []);

  const revealInHost = useCallback((path: string) => {
    void revealInFileManager(path).catch((reason: unknown) => {
      useToastStore.getState().showToast(
        i18n.t("explorer.revealFailed", {
          message: reason instanceof Error ? reason.message : String(reason),
        }),
        "error",
      );
    });
  }, []);

  // The tile menu mirrors the explorer's wording and its recycle semantics: on
  // Windows the delete goes to the Recycle Bin, elsewhere it is a plain unlink.
  const menuItems = (): ContextMenuItem[] => {
    if (!menu) return [];
    const entry = menu.entry;
    return [
      {
        id: "open",
        label: i18n.t("explorer.open"),
        onClick: () => openEntry(entry),
      },
      ...(hostOs === "windows"
        ? [
            {
              id: "reveal",
              label: i18n.t("explorer.reveal"),
              onClick: () => revealInHost(entry.path),
            } satisfies ContextMenuItem,
          ]
        : []),
      { id: "sep-1", separator: true },
      ...(selected.size <= 1
        ? [
            {
              id: "rename",
              label: i18n.t("explorer.rename"),
              onClick: () => setRenamingPath(entry.path),
            } satisfies ContextMenuItem,
            { id: "sep-2", separator: true } satisfies ContextMenuItem,
          ]
        : []),
      {
        id: "delete",
        label: i18n.t("explorer.delete"),
        danger: true,
        onClick: () =>
          selected.size > 1 ? trashSelected() : trashEntry(entry),
      },
    ];
  };

  const openMenu = useCallback(
    (entry: MediaEntry, at: { x: number; y: number }) => {
      setMenu({ x: at.x, y: at.y, entry });
    },
    [],
  );

  // Trash the file, then drop the tile right away: the server patches its
  // snapshot rather than re-walking, so nothing else has to catch up.
  const trashEntry = useCallback((entry: MediaEntry) => {
    void deleteEntries([entry.path], confirmDelete).then((removed) => {
      if (!removed) return;
      setListing((current) =>
        current
          ? {
              ...current,
              entries: current.entries.filter((item) => item.path !== entry.path),
            }
          : current,
      );
      setSelected((current) => {
        if (!current.has(entry.path)) return current;
        const next = new Set(current);
        next.delete(entry.path);
        return next;
      });
      setAnchor((current) => (current === entry.path ? null : current));
      setActive((current) => (current === entry.path ? null : current));
    });
  }, [confirmDelete]);

  const trashSelected = useCallback(() => {
    const paths = selectedEntries.map((entry) => entry.path);
    if (paths.length === 0) return;
    const gone = new Set(paths);
    void deleteEntries(paths, confirmDelete).then((removed) => {
      if (!removed) return;
      setListing((current) =>
        current
          ? {
              ...current,
              entries: current.entries.filter((item) => !gone.has(item.path)),
            }
          : current,
      );
      setSelected(new Set<string>());
      setAnchor(null);
      setActive(null);
    });
  }, [confirmDelete, selectedEntries]);

  const handleGalleryKeyDown = useCallback(
    (entry: MediaEntry, event: KeyboardEvent<HTMLElement>) => {
      const key = event.key;
      const mod = event.ctrlKey || event.metaKey;
      const paths = selectableEntries.map((item) => item.path);
      const index = selectableEntries.findIndex(
        (item) => item.path === entry.path,
      );

      if (mod && key.toLowerCase() === "a") {
        event.preventDefault();
        setSelected(new Set(paths));
        setAnchor(active ?? paths[0] ?? null);
        return;
      }
      if (mod && key.toLowerCase() === "d") {
        event.preventDefault();
        clearSelection();
        return;
      }
      if (key === "Escape") {
        if (selected.size > 0) event.preventDefault();
        clearSelection();
        return;
      }
      if (key === "Delete") {
        if (selected.size > 0) {
          event.preventDefault();
          trashSelected();
        }
        return;
      }
      if (key.toLowerCase() === "x") {
        event.preventDefault();
        const result = applyListSelection(selected, entry.path, anchor, {
          additive: true,
          visible: paths,
        });
        setSelected(result.selected);
        setAnchor(result.anchor);
        setActive(entry.path);
        return;
      }

      const moveTo = (nextIndex: number, extend: boolean) => {
        event.preventDefault();
        const next = selectableEntries[nextIndex];
        if (!next) return;
        if (extend) {
          const base = anchor ?? entry.path;
          const result = applyListSelection(selected, next.path, base, {
            range: true,
            visible: paths,
          });
          setSelected(result.selected);
          setAnchor(result.anchor);
        } else if (!mod) {
          const result = applyListSelection(selected, next.path, anchor, {
            visible: paths,
          });
          setSelected(result.selected);
          setAnchor(result.anchor);
        }
        setActive(next.path);
        focusEntry(next.path);
      };

      if (key === "Home") {
        moveTo(0, event.shiftKey);
        return;
      }
      if (key === "End") {
        moveTo(selectableEntries.length - 1, event.shiftKey);
        return;
      }
      if (key === "ArrowLeft") {
        moveTo(index - 1, event.shiftKey);
        return;
      }
      if (key === "ArrowRight") {
        moveTo(index + 1, event.shiftKey);
        return;
      }
      if (key === "ArrowUp" || key === "ArrowDown") {
        if (view === "list") {
          moveTo(index + (key === "ArrowUp" ? -1 : 1), event.shiftKey);
          return;
        }
        const node = Array.from(
          scrollRef.current?.querySelectorAll<HTMLElement>(
            "[data-gallery-path]",
          ) ?? [],
        ).find((candidate) => candidate.dataset.galleryPath === entry.path);
        const grid = node?.closest(".gallery-grid");
        const columns = grid
          ? Math.max(
              1,
              getComputedStyle(grid)
                .gridTemplateColumns.split(" ")
                .filter(Boolean).length,
            )
          : 1;
        moveTo(
          index + (key === "ArrowUp" ? -columns : columns),
          event.shiftKey,
        );
      }
    },
    [
      active,
      anchor,
      clearSelection,
      focusEntry,
      selected,
      selectableEntries,
      trashSelected,
      view,
    ],
  );

  const submitRename = useCallback((entry: MediaEntry, nextName: string) => {
    setRenamingPath(null);
    const name = nextName.trim();
    if (!name || name === entry.name) return;
    const problem = validateFileName(name);
    if (problem) {
      useToastStore.getState().showToast(i18n.t(problem), "error");
      return;
    }
    void renameEntry(entry.path, name)
      .then((next) => {
        if (!next) return;
        // The grid follows the rename at once instead of waiting for the
        // listing round-trip; the watcher then confirms the same state.
        setListing((current) =>
          current
            ? {
                ...current,
                entries: current.entries.map((item) =>
                  item.path === entry.path ? renamedEntry(item, next) : item,
                ),
              }
            : current,
        );
        // Keep the footer on the file the user just renamed.
        setSelected((current) => {
          if (!current.has(entry.path)) return current;
          const updated = new Set(current);
          updated.delete(entry.path);
          updated.add(next);
          return updated;
        });
        setAnchor((current) => (current === entry.path ? next : current));
        setActive((current) => (current === entry.path ? next : current));
      })
      .catch((reason: unknown) => {
        useToastStore.getState().showToast(
          i18n.t("explorer.renameFailed", {
            message: reason instanceof Error ? reason.message : String(reason),
          }),
          "error",
        );
      });
  }, []);

  const changeMode = useCallback(
    (value: GalleryMode) => {
      setMode(value);
      clearSelection();
    },
    [clearSelection],
  );

  const toggleSection = useCallback((key: string) => {
    setFoldedKinds((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const scopeCount = folder
    ? (folderIndex.get(folder)?.count ?? 0)
    : filteredEntries.length;
  const trail = folderTrail(folder);
  const emptyWorkspace = !loading && !error && entries.length === 0;
  const emptySearch =
    !loading && !error && entries.length > 0 && filteredEntries.length === 0;
  const emptyFolder =
    !emptySearch &&
    mode === "folder" &&
    folders.length === 0 &&
    sections.length === 0;
  const selectedBytes = selectedEntries.reduce(
    (total, entry) => total + entry.size,
    0,
  );
  const openActive = useCallback(() => {
    if (activeEntry) openEntry(activeEntry);
  }, [activeEntry, openEntry]);
  const renameActive = useCallback(() => {
    if (activeEntry) setRenamingPath(activeEntry.path);
  }, [activeEntry]);
  const revealActive = useCallback(() => {
    if (activeEntry) revealInHost(activeEntry.path);
  }, [activeEntry, revealInHost]);

  return (
    <div className="gallery-panel">
      <div className="gallery-toolbar">
        <div className="gallery-modes" role="tablist" aria-label={i18n.t("gallery.title")}>
          {MODES.map((value) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={mode === value}
              className={`gallery-mode${mode === value ? " is-active" : ""}`}
              onClick={() => changeMode(value)}
            >
              {i18n.t(MODE_LABEL[value])}
            </button>
          ))}
        </div>
        <div className="gallery-toolbar-actions">
          <div
            className="gallery-views"
            role="group"
            aria-label={i18n.t("gallery.view.label")}
          >
            <button
              type="button"
              className={"gallery-view" + (view === "grid" ? " is-active" : "")}
              title={i18n.t("gallery.view.grid")}
              aria-label={i18n.t("gallery.view.grid")}
              aria-pressed={view === "grid"}
              onClick={() => setView("grid")}
            >
              <SquaresFour size={13} />
            </button>
            <button
              type="button"
              className={"gallery-view" + (view === "list" ? " is-active" : "")}
              title={i18n.t("gallery.view.list")}
              aria-label={i18n.t("gallery.view.list")}
              aria-pressed={view === "list"}
              onClick={() => setView("list")}
            >
              <ListBullets size={13} />
            </button>
          </div>
          <button
            type="button"
            className="gallery-refresh"
            title={i18n.t("gallery.refresh")}
            aria-label={i18n.t("gallery.refresh")}
            onClick={() => setRequest((value) => ({ n: value.n + 1, fresh: true }))}
          >
            <ArrowClockwise
              size={14}
              className={loading ? "gallery-spin" : undefined}
            />
          </button>
        </div>
      </div>

      <div className="gallery-filters">
        <label className="gallery-search">
          <MagnifyingGlass size={12} />
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={i18n.t("gallery.search")}
            aria-label={i18n.t("gallery.search")}
            spellCheck={false}
          />
          {query ? (
            <button
              type="button"
              className="gallery-search-clear"
              title={i18n.t("gallery.searchClear")}
              aria-label={i18n.t("gallery.searchClear")}
              onClick={() => setQuery("")}
            >
              <X size={10} />
            </button>
          ) : null}
        </label>
        <div className="gallery-sort">
          <select
            value={sort}
            aria-label={i18n.t("gallery.sort.label")}
            title={i18n.t("gallery.sort.label")}
            onChange={(event) => setSort(event.target.value as GallerySort)}
          >
            <option value="name">{i18n.t("gallery.sort.name")}</option>
            <option value="date">{i18n.t("gallery.sort.date")}</option>
            <option value="size">{i18n.t("gallery.sort.size")}</option>
            <option value="kind">{i18n.t("gallery.sort.kind")}</option>
          </select>
          <button
            type="button"
            className="gallery-sort-dir"
            title={
              sortDir === "asc"
                ? i18n.t("gallery.sort.asc")
                : i18n.t("gallery.sort.desc")
            }
            aria-label={
              sortDir === "asc"
                ? i18n.t("gallery.sort.asc")
                : i18n.t("gallery.sort.desc")
            }
            onClick={() =>
              setSortDir((current) => (current === "asc" ? "desc" : "asc"))
            }
          >
            {sortDir === "asc" ? (
              <SortAscending size={13} />
            ) : (
              <SortDescending size={13} />
            )}
          </button>
        </div>
      </div>

      {listing ? (
        <div className="gallery-meta">
          <span>{i18n.t("gallery.count", { count: scopeCount })}</span>
          {folderIndex.size > 0 ? (
            <span>{i18n.t("gallery.folders", { count: folderIndex.size })}</span>
          ) : null}
          {listing.truncated ? (
            <span className="gallery-meta-warn">{i18n.t("gallery.truncated")}</span>
          ) : null}
        </div>
      ) : null}

      {mode === "folder" ? (
        <nav className="gallery-crumbs" aria-label={i18n.t("gallery.root")}>
          {trail.map((path, index) => (
            <span key={path || "root"} className="gallery-crumb">
              {index > 0 ? <CaretRight size={9} weight="bold" /> : null}
              <button
                type="button"
                disabled={path === folder}
                onClick={() => enterFolder(path)}
              >
                {path ? folderLabel(path) : i18n.t("gallery.root")}
              </button>
            </span>
          ))}
        </nav>
      ) : null}

      <div className="gallery-scroll" ref={scrollRef}>
        {error ? (
          <div className="gallery-error" role="alert">
            <span className="gallery-error-title">{i18n.t("gallery.error")}</span>
            <span className="gallery-error-detail">{error}</span>
            <button
              type="button"
              onClick={() => setRequest((value) => ({ n: value.n + 1, fresh: true }))}
            >
              {i18n.t("gallery.retry")}
            </button>
          </div>
        ) : null}

        {loading && !listing ? (
          <div className="gallery-placeholder">
            <Images size={22} weight="thin" />
            <span>{i18n.t("gallery.loading")}</span>
          </div>
        ) : null}

        {mode === "folder" && folders.length > 0 ? (
          <div
            className={
              view === "list"
                ? "gallery-list gallery-folder-list"
                : "gallery-grid gallery-folder-grid"
            }
          >
            {folders.map((item) =>
              view === "list" ? (
                <button
                  key={item.path}
                  type="button"
                  className="gallery-row"
                  title={item.path}
                  onClick={() => enterFolder(item.path)}
                >
                  <span className="gallery-row-frame gallery-row-folder">
                    <FolderSimple size={14} weight="thin" />
                  </span>
                  <span className="gallery-row-name">{item.name}</span>
                  <span className="gallery-row-meta">{item.count}</span>
                  <CaretRight size={10} />
                </button>
              ) : (
                <button
                  key={item.path}
                  type="button"
                  className="gallery-tile gallery-folder-tile"
                  title={item.path}
                  onClick={() => enterFolder(item.path)}
                >
                  <span className="gallery-tile-frame">
                    <span className="gallery-tile-face gallery-tile-folder">
                      <FolderSimple size={22} weight="thin" />
                    </span>
                    <span className="gallery-tile-ext">{item.count}</span>
                  </span>
                  <span className="gallery-tile-name">{item.name}</span>
                </button>
              ),
            )}
          </div>
        ) : null}

        {visibleSections.length > 0 ? (
          <GalleryGrid
            sections={visibleSections}
            view={view}
            folded={foldedKinds}
            onToggle={toggleSection}
            ariaLabel={i18n.t("gallery.title")}
            selected={selected}
            active={active}
            renaming={renamingPath}
            onSelect={selectEntry}
            onActivate={activateEntry}
            onKeyDown={handleGalleryKeyDown}
            onOpen={openEntry}
            onMenu={openMenu}
            onRename={submitRename}
            onRenameCancel={() => setRenamingPath(null)}
          />
        ) : null}

        {emptyWorkspace ? (
          <div className="gallery-placeholder">
            <Images size={22} weight="thin" />
            <span>{i18n.t("gallery.empty")}</span>
            <span className="gallery-placeholder-hint">
              {i18n.t("gallery.emptyHint")}
            </span>
          </div>
        ) : null}

        {!loading && !emptyWorkspace && emptySearch ? (
          <div className="gallery-placeholder">
            <span>{i18n.t("gallery.noMatches")}</span>
            <span className="gallery-placeholder-hint">
              {i18n.t("gallery.noMatchesHint")}
            </span>
          </div>
        ) : null}

        {!loading && !emptyWorkspace && emptyFolder ? (
          <div className="gallery-placeholder">
            <span>{i18n.t("gallery.emptyFolder")}</span>
          </div>
        ) : null}

        {hasMore ? <div className="gallery-sentinel" ref={sentinelRef} /> : null}
      </div>

      {selectedEntries.length > 0 && activeEntry ? (
        <footer className="gallery-detail">
          <div className="gallery-detail-text">
            <div className="gallery-detail-name" title={activeEntry.path}>
              {selectedEntries.length > 1
                ? i18n.t("gallery.selected", { count: selectedEntries.length })
                : activeEntry.name}
            </div>
            <div className="gallery-detail-meta">
              {selectedEntries.length > 1 ? (
                <span>{formatBytes(selectedBytes)}</span>
              ) : (
                <>
                  <span>{kindLabel(activeEntry.kind)}</span>
                  <span>{formatBytes(activeEntry.size)}</span>
                  {activeEntry.folder ? (
                    <span>{activeEntry.folder}</span>
                  ) : null}
                </>
              )}
            </div>
          </div>
          <div className="gallery-detail-actions">
            <button
              type="button"
              className="gallery-detail-action"
              onClick={openActive}
            >
              {i18n.t("explorer.open")}
            </button>
            {hostOs === "windows" ? (
              <button
                type="button"
                className="gallery-detail-action"
                onClick={revealActive}
              >
                {i18n.t("explorer.reveal")}
              </button>
            ) : null}
            {selectedEntries.length === 1 ? (
              <button
                type="button"
                className="gallery-detail-action"
                onClick={renameActive}
              >
                {i18n.t("explorer.rename")}
              </button>
            ) : null}
            <button
              type="button"
              className="gallery-detail-action is-danger"
              onClick={trashSelected}
            >
              {i18n.t("explorer.delete")}
            </button>
          </div>
        </footer>
      ) : null}

      {menu ? (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems()}
          onClose={() => setMenu(null)}
        />
      ) : null}
      {dialog}
    </div>
  );
}
