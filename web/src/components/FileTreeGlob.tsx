import { useEffect, useMemo, useState } from "react";
import { MagnifyingGlass, X } from "@phosphor-icons/react";

import { fetchGlob, type TreeEntry } from "../api/workspace";
import { gitFileLetters, gitStatusColor } from "../lib/gitStatus";
import { useEditorStore } from "../stores/editorStore";
import { useGitStore } from "../stores/gitStore";
import { useTreeStore } from "../stores/treeStore";
import { getFileIcon, FolderIcon } from "../utils/fileIcon";
import { parentPath } from "../utils/path";

const DEBOUNCE_MS = 200;

export function FileTreeGlobInput({
  query,
  onQueryChange,
}: {
  query: string;
  onQueryChange: (q: string) => void;
}) {
  return (
    <div
      className="flex items-center gap-1 rounded border border-(--_dk-line) bg-(--_dk-editor) px-1.5"
      role="search"
      onClick={(e) => e.stopPropagation()}
    >
      <MagnifyingGlass size={12} className="shrink-0 text-(--_dk-text-muted)" />
      <input
        value={query}
        onChange={(e) => onQueryChange(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") {
            e.preventDefault();
            onQueryChange("");
          }
        }}
        placeholder="Filter files"
        title="Filter by file name. Globs like *.ts or src/**/*.rs work too."
        className="min-w-0 flex-1 border-0 bg-transparent py-0.5 text-xs text-(--_dk-text-secondary) outline-none placeholder:text-(--_dk-text-disabled)"
      />
      {query ? (
        <button
          type="button"
          title="Clear filter"
          className="shrink-0 text-(--_dk-text-muted) hover:text-(--_dk-text-secondary)"
          onClick={() => onQueryChange("")}
        >
          <X size={12} />
        </button>
      ) : null}
    </div>
  );
}

export function FileTreeGlobHits({
  query,
  onClear,
}: {
  query: string;
  onClear: () => void;
}) {
  const [hits, setHits] = useState<TreeEntry[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const gitStatus = useGitStore((s) => s.status);
  const gitLetters = useMemo(() => gitFileLetters(gitStatus), [gitStatus]);

  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      setBusy(true);
      void fetchGlob(query)
        .then((listing) => {
          if (cancelled) return;
          setHits(listing.entries);
          setTruncated(listing.truncated);
          setError(null);
        })
        .catch((e: unknown) => {
          if (cancelled) return;
          setHits([]);
          setTruncated(false);
          setError(e instanceof Error ? e.message : String(e));
        })
        .finally(() => {
          if (!cancelled) setBusy(false);
        });
    }, DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query]);

  const openHit = (entry: TreeEntry) => {
    if (entry.kind === "dir") {
      onClear();
      void useTreeStore.getState().revealPath(entry.path).then(() => {
        void useTreeStore.getState().expandDir(entry.path);
      });
      return;
    }
    void useEditorStore.getState().openFile(entry.path);
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto py-1">
      {error && <p className="px-3 py-2 text-xs text-(--_dk-red-500)">{error}</p>}
      {!error && !busy && hits.length === 0 && (
        <p className="px-3 py-2 text-xs text-(--_dk-text-disabled)">
          No files matching &lsquo;{query}&rsquo;
        </p>
      )}
      {hits.map((entry) => (
        <GlobHitRow
          key={entry.path}
          entry={entry}
          gitLetter={gitLetters.get(entry.path) ?? null}
          onOpen={() => openHit(entry)}
        />
      ))}
      {truncated && (
        <p className="px-3 py-2 text-[11px] text-(--_dk-text-disabled)">
          Showing first 1000 matches
        </p>
      )}
    </div>
  );
}

function GlobHitRow({
  entry,
  gitLetter,
  onOpen,
}: {
  entry: TreeEntry;
  gitLetter: string | null;
  onOpen: () => void;
}) {
  const isDir = entry.kind === "dir";
  const dir = parentPath(entry.path);
  const activePath = useEditorStore((s) => s.activePath);
  const isActive = !isDir && activePath === entry.path;
  const Glyph = isDir ? FolderIcon : getFileIcon(entry.name);

  return (
    <button
      type="button"
      title={entry.path}
      onClick={onOpen}
      className={`flex w-full cursor-default items-center gap-1 truncate px-2 py-0.5 text-left text-sm transition-colors hover:bg-(--_dk-ix-bg-hover) ${
        isActive
          ? "bg-(--_dk-ix-bg-selected) text-(--_dk-text-secondary)"
          : "text-(--_dk-text-secondary)"
      }`}
    >
      <Glyph
        size={16}
        weight="regular"
        aria-hidden
        className="h-4 w-4 shrink-0 select-none text-(--_dk-fg-muted)"
      />
      <span className="min-w-0 truncate">
        {entry.name}
        {dir ? (
          <span className="ml-1.5 text-[11px] text-(--_dk-text-disabled)">{dir}</span>
        ) : null}
      </span>
      {gitLetter && (
        <span className={`ml-auto font-mono text-[11px] ${gitStatusColor(gitLetter)}`}>
          {gitLetter}
        </span>
      )}
    </button>
  );
}
