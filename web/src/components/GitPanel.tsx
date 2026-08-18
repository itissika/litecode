import { type MouseEvent, type ReactNode, useEffect, useMemo, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  GitBranch,
  Minus,
  Plus,
  ArrowCounterClockwise,
} from "@phosphor-icons/react";

import type { GitCommitInfo, GitFile } from "../api/workspace";
import { useEditorStore } from "../stores/editorStore";
import {
  gitRowId,
  selectedPaths,
  useGitStore,
  type GitSection,
} from "../stores/gitStore";
import { fileNameFromPath } from "../utils/language";

function statusLabel(letter: string, untracked: boolean): string {
  if (untracked) return "U";
  return letter || "M";
}

function confirmDiscard(paths: string[]): boolean {
  if (paths.length === 0) return false;
  const preview = paths.slice(0, 8).join("\n");
  const extra = paths.length > 8 ? `\n…and ${paths.length - 8} more` : "";
  return window.confirm(`Discard changes to:\n${preview}${extra}`);
}

function SectionHeader({
  title,
  count,
  actions,
}: {
  title: string;
  count: number;
  actions: ReactNode;
}) {
  return (
    <div className="flex items-center gap-1 px-2 py-1 text-dk-xs text-(--_dk-text-muted)">
      <span className="min-w-0 flex-1 truncate uppercase tracking-wide">
        {title}
        {count > 0 ? ` (${count})` : ""}
      </span>
      {actions}
    </div>
  );
}

function IconBtn({
  title,
  disabled,
  onClick,
  children,
}: {
  title: string;
  disabled?: boolean;
  onClick: (e: MouseEvent) => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className="rounded p-0.5 text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-selected) hover:text-(--_dk-text-secondary) disabled:opacity-40"
    >
      {children}
    </button>
  );
}

function FileRow({
  section,
  file,
  visible,
}: {
  section: GitSection;
  file: GitFile;
  visible: string[];
}) {
  const id = gitRowId(section, file.path);
  const selected = useGitStore((s) => s.selected.has(id));
  const select = useGitStore((s) => s.select);
  const stagePaths = useGitStore((s) => s.stagePaths);
  const unstagePaths = useGitStore((s) => s.unstagePaths);
  const restorePaths = useGitStore((s) => s.restorePaths);
  const mutating = useGitStore((s) => s.mutating);
  const openFile = useEditorStore((s) => s.openFile);

  const onClick = (e: MouseEvent) => {
    select(id, {
      additive: e.ctrlKey || e.metaKey,
      range: e.shiftKey,
      visible,
    });
    if (e.ctrlKey || e.metaKey || e.shiftKey) return;
    void openFile(file.path);
  };

  return (
    <div
      role="option"
      aria-selected={selected}
      onClick={onClick}
      className={`group flex cursor-pointer items-center gap-1 px-2 py-0.5 text-dk-xs ${
        selected
          ? "bg-(--_dk-ix-bg-selected) text-(--_dk-text-secondary)"
          : "text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-selected)/50"
      }`}
      title={file.orig_path ? `${file.orig_path} → ${file.path}` : file.path}
    >
      <span className="w-3 shrink-0 font-mono text-(--_dk-text-muted)">
        {statusLabel(file.status, file.untracked)}
      </span>
      <span className="min-w-0 flex-1 truncate">{fileNameFromPath(file.path)}</span>
      <span className="hidden shrink-0 gap-0.5 group-hover:flex">
        {section === "changes" ? (
          <IconBtn
            title="Stage"
            disabled={mutating}
            onClick={(e) => {
              e.stopPropagation();
              void stagePaths([file.path]);
            }}
          >
            <Plus size={12} />
          </IconBtn>
        ) : (
          <IconBtn
            title="Unstage"
            disabled={mutating}
            onClick={(e) => {
              e.stopPropagation();
              void unstagePaths([file.path]);
            }}
          >
            <Minus size={12} />
          </IconBtn>
        )}
        <IconBtn
          title="Discard"
          disabled={mutating}
          onClick={(e) => {
            e.stopPropagation();
            if (!confirmDiscard([file.path])) return;
            void restorePaths([file.path]);
          }}
        >
          <ArrowCounterClockwise size={12} />
        </IconBtn>
      </span>
    </div>
  );
}

function CommitRow({ commit }: { commit: GitCommitInfo }) {
  const [hover, setHover] = useState(false);
  const short = commit.sha.slice(0, 7);
  const tip = [commit.author, commit.date, "", commit.subject, commit.body]
    .filter((line, i, arr) => !(line === "" && i === arr.length - 1))
    .join("\n");

  return (
    <div
      className="relative px-2 py-1 text-dk-xs text-(--_dk-text-secondary)"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <div className="flex gap-1.5">
        <span className="shrink-0 font-mono text-(--_dk-text-muted)">{short}</span>
        <span className="min-w-0 truncate">{commit.subject}</span>
      </div>
      {hover && (
        <div className="absolute left-2 right-2 z-20 mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded border border-(--_dk-line-visible) bg-(--_dk-overlay) p-2 text-(--_dk-text-secondary) shadow-[0_6px_18px_rgba(0,0,0,0.18)]">
          {tip}
        </div>
      )}
    </div>
  );
}

export function GitPanel() {
  const status = useGitStore((s) => s.status);
  const commits = useGitStore((s) => s.commits);
  const message = useGitStore((s) => s.message);
  const selected = useGitStore((s) => s.selected);
  const loading = useGitStore((s) => s.loading);
  const mutating = useGitStore((s) => s.mutating);
  const error = useGitStore((s) => s.error);
  const setMessage = useGitStore((s) => s.setMessage);
  const refresh = useGitStore((s) => s.refresh);
  const stagePaths = useGitStore((s) => s.stagePaths);
  const unstagePaths = useGitStore((s) => s.unstagePaths);
  const restorePaths = useGitStore((s) => s.restorePaths);
  const commit = useGitStore((s) => s.commit);
  const pull = useGitStore((s) => s.pull);
  const push = useGitStore((s) => s.push);

  useEffect(() => {
    void refresh({ silent: false });
  }, [refresh]);

  const visibleIds = useMemo(() => {
    return [
      ...status.staged.map((f) => gitRowId("staged", f.path)),
      ...status.changes.map((f) => gitRowId("changes", f.path)),
    ];
  }, [status.staged, status.changes]);

  const selectedStaged = selectedPaths(selected, "staged");
  const selectedChanges = selectedPaths(selected, "changes");
  const selectedAny = [...new Set([...selectedStaged, ...selectedChanges])];
  const busy = mutating;
  const upstream =
    status.upstream_ahead || status.upstream_behind
      ? ` ↑${status.upstream_ahead} ↓${status.upstream_behind}`
      : "";

  if (!status.is_repo && !loading) {
    return (
      <div className="flex h-full flex-col text-dk-xs text-(--_dk-text-muted)">
        <div className="px-3 py-3">This workspace is not a git repository.</div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col text-dk-xs">
      <div className="flex items-center gap-1 border-b border-(--_dk-line) px-2 py-1">
        <GitBranch size={14} className="shrink-0 text-(--_dk-text-muted)" />
        <span className="min-w-0 flex-1 truncate text-(--_dk-text-secondary)">
          {status.branch ?? "HEAD"}
          {upstream}
        </span>
        <IconBtn title="Pull" disabled={busy} onClick={() => void pull()}>
          <ArrowDown size={14} />
        </IconBtn>
        <IconBtn title="Push" disabled={busy} onClick={() => void push()}>
          <ArrowUp size={14} />
        </IconBtn>
      </div>

      <div className="border-b border-(--_dk-line) p-2">
        <textarea
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          placeholder="Message (Ctrl+Enter to commit)"
          rows={3}
          className="w-full resize-none rounded border border-(--_dk-line-visible) bg-(--_dk-surface-header) px-2 py-1 text-(--_dk-text-secondary) outline-none"
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
              e.preventDefault();
              void commit();
            }
          }}
        />
        <div className="mt-1 flex justify-end">
          <button
            type="button"
            disabled={busy || !message.trim() || status.staged.length === 0}
            onClick={() => void commit()}
            className="rounded bg-(--_dk-ix-bg-selected) px-2 py-0.5 text-(--_dk-text-secondary) disabled:opacity-40"
          >
            Commit
          </button>
        </div>
      </div>

      {error && (
        <div className="border-b border-(--_dk-red-500) px-2 py-1 text-(--_dk-red-500)">
          {error}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-auto">
        <SectionHeader
          title="Staged Changes"
          count={status.staged.length}
          actions={
            <>
              <IconBtn
                title="Unstage selected"
                disabled={busy || selectedStaged.length === 0}
                onClick={() => void unstagePaths(selectedStaged)}
              >
                <Minus size={12} />
              </IconBtn>
              <IconBtn
                title="Unstage all"
                disabled={busy || status.staged.length === 0}
                onClick={() => void unstagePaths(status.staged.map((f) => f.path))}
              >
                <Minus size={14} weight="bold" />
              </IconBtn>
            </>
          }
        />
        {status.staged.map((file) => (
          <FileRow
            key={gitRowId("staged", file.path)}
            section="staged"
            file={file}
            visible={visibleIds}
          />
        ))}

        <SectionHeader
          title="Changes"
          count={status.changes.length}
          actions={
            <>
              <IconBtn
                title="Stage selected"
                disabled={busy || selectedChanges.length === 0}
                onClick={() => void stagePaths(selectedChanges)}
              >
                <Plus size={12} />
              </IconBtn>
              <IconBtn
                title="Stage all"
                disabled={busy || status.changes.length === 0}
                onClick={() => void stagePaths(status.changes.map((f) => f.path))}
              >
                <Plus size={14} weight="bold" />
              </IconBtn>
              <IconBtn
                title="Discard selected"
                disabled={busy || selectedAny.length === 0}
                onClick={() => {
                  if (!confirmDiscard(selectedAny)) return;
                  void restorePaths(selectedAny);
                }}
              >
                <ArrowCounterClockwise size={12} />
              </IconBtn>
              <IconBtn
                title="Discard all"
                disabled={busy || status.changes.length === 0}
                onClick={() => {
                  const paths = status.changes.map((f) => f.path);
                  if (!confirmDiscard(paths)) return;
                  void restorePaths(paths);
                }}
              >
                <ArrowCounterClockwise size={14} weight="bold" />
              </IconBtn>
            </>
          }
        />
        {status.changes.map((file) => (
          <FileRow
            key={gitRowId("changes", file.path)}
            section="changes"
            file={file}
            visible={visibleIds}
          />
        ))}

        <SectionHeader title="Commits" count={commits.length} actions={null} />
        {commits.map((c) => (
          <CommitRow key={c.sha} commit={c} />
        ))}
      </div>
    </div>
  );
}
