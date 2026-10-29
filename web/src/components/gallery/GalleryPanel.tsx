import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import {
  ArrowClockwise,
  ArrowSquareOut,
  CaretRight,
  FolderSimple,
  Images,
} from "@phosphor-icons/react";

import {
  fetchMedia,
  openWithDefaultApp,
  type MediaEntry,
  type MediaKind,
  type MediaListing,
} from "../../api/workspace";
import { formatBytes } from "../../lib/bytes";
import { useEditorStore } from "../../stores/editorStore";
import { useToastStore } from "../../stores/toastStore";
import i18n from "../../i18n";
import { GalleryGrid } from "./GalleryGrid";
import {
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
} from "./galleryGroups";

const MODES: readonly GalleryMode[] = ["all", "folder", "kind"];
const MODE_LABEL: Record<GalleryMode, string> = {
  all: "gallery.mode.all",
  folder: "gallery.mode.folder",
  kind: "gallery.mode.kind",
};
const PAGE = 120;
const EMPTY_ENTRIES: MediaEntry[] = [];

function kindLabel(kind: MediaKind): string {
  return i18n.t(`gallery.kind.${kind}`);
}

/**
 * Workspace media browser. One server walk feeds three groupings: a flat grid,
 * a folder drill-down, and image / video / audio sections. Thumbnails are
 * fetched lazily and cached by path + mtime; double click hands the original to
 * the system player.
 */
export function GalleryPanel(_props: IDockviewPanelProps) {
  const [listing, setListing] = useState<MediaListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  const [mode, setMode] = useState<GalleryMode>("all");
  const [folder, setFolder] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [limit, setLimit] = useState(PAGE);

  const scrollRef = useRef<HTMLDivElement | null>(null);
  const sentinelRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError(null);
    void fetchMedia()
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
  }, [reloadToken]);

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

  const visibleSections = useMemo(
    () => capSections(sections, limit),
    [limit, sections],
  );
  const hasMore = useMemo(
    () => sectionsHaveMore(sections, limit),
    [limit, sections],
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
  }, [hasMore, limit, mode, folder]);

  const openEntry = useCallback((entry: MediaEntry) => {
    void openWithDefaultApp(entry.path).catch((reason: unknown) => {
      useToastStore.getState().showToast(
        i18n.t("gallery.openFailed", {
          message: reason instanceof Error ? reason.message : String(reason),
        }),
        "error",
      );
    });
  }, []);

  const openInEditor = useCallback((entry: MediaEntry) => {
    void useEditorStore.getState().openFile(entry.path);
  }, []);

  const enterFolder = useCallback((path: string) => {
    setFolder(path);
    setSelected(null);
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
        <button
          type="button"
          className="gallery-refresh"
          title={i18n.t("gallery.refresh")}
          aria-label={i18n.t("gallery.refresh")}
          onClick={() => setReloadToken((value) => value + 1)}
        >
          <ArrowClockwise
            size={14}
            className={loading ? "gallery-spin" : undefined}
          />
        </button>
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
            <button type="button" onClick={() => setReloadToken((value) => value + 1)}>
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
          <div className="gallery-folder-list">
            {folders.map((item) => (
              <button
                key={item.path}
                type="button"
                className="gallery-folder"
                title={item.path}
                onClick={() => enterFolder(item.path)}
              >
                <FolderSimple size={15} weight="thin" />
                <span className="gallery-folder-name">{item.name}</span>
                <span className="gallery-folder-count">{item.count}</span>
                <CaretRight size={10} />
              </button>
            ))}
          </div>
        ) : null}

        {visibleSections.length > 0 ? (
          <GalleryGrid
            sections={visibleSections}
            selected={selected}
            onSelect={(entry) => setSelected(entry.path)}
            onOpen={openEntry}
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
          <div className="gallery-detail-actions">
            <button type="button" onClick={() => openEntry(selectedEntry)}>
              <ArrowSquareOut size={12} />
              {i18n.t("gallery.open")}
            </button>
            <button type="button" onClick={() => openInEditor(selectedEntry)}>
              {i18n.t("gallery.openInEditor")}
            </button>
          </div>
        </footer>
      ) : null}
    </div>
  );
}
