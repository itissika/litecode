import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import {
  ArrowClockwise,
  CaretRight,
  FolderSimple,
  Images,
  ListBullets,
  SquaresFour,
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
import { FileTreeContextMenu, type FileTreeMenuItem } from "../FileTreeContextMenu";
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
  kindSections,
  sectionsHaveMore,
  type GalleryMode,
  type GallerySection,
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
  const [listing, setListing] = useState<MediaListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [request, setRequest] = useState<ListingRequest>({ n: 0, fresh: false });

  const [mode, setMode] = useState<GalleryMode>("all");
  const [view, setView] = useState<GalleryView>(loadView);
  const [folder, setFolder] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
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
  const folderIndex = useMemo(() => buildFolderIndex(entries), [entries]);
  const folders = useMemo(
    () => childFolders(folderIndex, folder),
    [folderIndex, folder],
  );

  const sections = useMemo(() => {
    if (mode === "all") return allSection(entries);
    if (mode === "kind") return kindSections(entries, kindLabel);
    const direct = folderEntries(entries, folder);
    return direct.length ? [{ key: "folder", entries: direct, total: direct.length }] : [];
  }, [entries, folder, mode]);

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

  const selectedEntry = useMemo(
    () => entries.find((entry) => entry.path === selected) ?? null,
    [entries, selected],
  );

  // A reload may drop the drilled folder or the selected file.
  useEffect(() => {
    if (folder && !folderIndex.has(folder)) setFolder("");
  }, [folder, folderIndex]);
  useEffect(() => {
    if (selected && !entries.some((entry) => entry.path === selected)) {
      setSelected(null);
    }
  }, [entries, selected]);

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
    setSelected(null);
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
  const menuItems = (): FileTreeMenuItem[] => {
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
            } satisfies FileTreeMenuItem,
          ]
        : []),
      { id: "sep-1", separator: true },
      {
        id: "rename",
        label: i18n.t("explorer.rename"),
        onClick: () => setRenamingPath(entry.path),
      },
      { id: "sep-2", separator: true },
      {
        id: "delete",
        label: i18n.t("explorer.delete"),
        danger: true,
        onClick: () => trashEntry(entry),
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
    void deleteEntries([entry.path]).then((removed) => {
      if (!removed) return;
      setListing((current) =>
        current
          ? {
              ...current,
              entries: current.entries.filter((item) => item.path !== entry.path),
            }
          : current,
      );
      setSelected((current) => (current === entry.path ? null : current));
    });
  }, []);

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
        setSelected((current) => (current === entry.path ? next : current));
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
    : entries.length;
  const trail = folderTrail(folder);
  const emptyWorkspace = !loading && !error && entries.length === 0;
  const emptyFolder =
    mode === "folder" && folders.length === 0 && sections.length === 0;

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
              onClick={() => setMode(value)}
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
            selected={selected}
            renaming={renamingPath}
            onSelect={(entry) => setSelected(entry.path)}
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

        {!loading && !emptyWorkspace && emptyFolder ? (
          <div className="gallery-placeholder">
            <span>{i18n.t("gallery.emptyFolder")}</span>
          </div>
        ) : null}

        {hasMore ? <div className="gallery-sentinel" ref={sentinelRef} /> : null}
      </div>

      {selectedEntry ? (
        <footer className="gallery-detail">
          <div className="gallery-detail-text">
            <div className="gallery-detail-name" title={selectedEntry.path}>
              {selectedEntry.name}
            </div>
            <div className="gallery-detail-meta">
              <span>{kindLabel(selectedEntry.kind)}</span>
              <span>{formatBytes(selectedEntry.size)}</span>
              {selectedEntry.folder ? <span>{selectedEntry.folder}</span> : null}
            </div>
          </div>
        </footer>
      ) : null}

      {menu ? (
        <FileTreeContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems()}
          onClose={() => setMenu(null)}
        />
      ) : null}
    </div>
  );
}
