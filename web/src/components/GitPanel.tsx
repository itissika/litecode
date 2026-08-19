import { type MouseEvent, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  CaretDown,
  CaretRight,
  GitBranch,
  Minus,
  Plus,
  ArrowCounterClockwise,
} from "@phosphor-icons/react";

import { GitCommitGraph } from "./GitCommitGraph";
import type { GitCommitInfo, GitFile } from "../api/workspace";
import { FoldCard } from "./FoldCard";
import { FOLDCARD_HEADER_TONE } from "./FoldCard";
import { Popover } from "./ui/Popover";
import { useEditorStore } from "../stores/editorStore";
import {
  actionTargetPaths,
  gitRowId,
  useGitStore,
  type GitSection,
} from "../stores/gitStore";
import {
  buildGitTree,
  descendantFiles,
  visibleFileIds,
  type GitTreeNode,
} from "../lib/gitTree";
import {
  GIT_GRAPH_ROW_HEIGHT,
  graphWidth,
  layoutGitGraph,
  maxLanesForWidth,
} from "../lib/gitGraph";
import { fileNameFromPath } from "../utils/language";
import { FolderIcon, getFileIcon } from "../utils/fileIcon";
import { gitStatusColor, gitStatusLabel } from "../lib/gitStatus";

const SPLIT_KEY = "litecode-git-split";
const SPLIT_DEFAULT = 0.55;

function confirmDiscard(paths: string[]): boolean {
  if (paths.length === 0) return false;
  const preview = paths.slice(0, 8).join("\n");
  const extra = paths.length > 8 ? `\n…and ${paths.length - 8} more` : "";
  return window.confirm(`Discard changes to:\n${preview}${extra}`);
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

function FileActions({
  section,
  paths,
  mutating,
}: {
  section: GitSection;
  paths: string[];
  mutating: boolean;
}) {
  const stagePaths = useGitStore((s) => s.stagePaths);
  const unstagePaths = useGitStore((s) => s.unstagePaths);
  const restorePaths = useGitStore((s) => s.restorePaths);

  return (
    <span className="hidden shrink-0 gap-0.5 group-hover:flex">
      {section === "changes" ? (
        <IconBtn
          title="Stage"
          disabled={mutating || paths.length === 0}
          onClick={(e) => {
            e.stopPropagation();
            void stagePaths(paths);
          }}
        >
          <Plus size={12} />
        </IconBtn>
      ) : (
        <IconBtn
          title="Unstage"
          disabled={mutating || paths.length === 0}
          onClick={(e) => {
            e.stopPropagation();
            void unstagePaths(paths);
          }}
        >
          <Minus size={12} />
        </IconBtn>
      )}
      <IconBtn
        title="Discard"
        disabled={mutating || paths.length === 0}
        onClick={(e) => {
          e.stopPropagation();
          if (!confirmDiscard(paths)) return;
          void restorePaths(paths);
        }}
      >
        <ArrowCounterClockwise size={12} />
      </IconBtn>
    </span>
  );
}

function TreeNodes({
  nodes,
  section,
  depth,
  collapsed,
  toggleCollapsed,
  visible,
}: {
  nodes: GitTreeNode[];
  section: GitSection;
  depth: number;
  collapsed: Set<string>;
  toggleCollapsed: (path: string) => void;
  visible: string[];
}) {
  return (
    <>
      {nodes.map((node) =>
        node.kind === "dir" ? (
          <DirRow
            key={`dir:${node.path}`}
            node={node}
            section={section}
            depth={depth}
            collapsed={collapsed}
            toggleCollapsed={toggleCollapsed}
            visible={visible}
          />
        ) : (
          <FileRow
            key={gitRowId(section, node.file.path)}
            file={node.file}
            section={section}
            depth={depth}
            visible={visible}
          />
        ),
      )}
    </>
  );
}

function DirRow({
  node,
  section,
  depth,
  collapsed,
  toggleCollapsed,
  visible,
}: {
  node: Extract<GitTreeNode, { kind: "dir" }>;
  section: GitSection;
  depth: number;
  collapsed: Set<string>;
  toggleCollapsed: (path: string) => void;
  visible: string[];
}) {
  const mutating = useGitStore((s) => s.mutating);
  const open = !collapsed.has(node.path);
  const files = descendantFiles(node);
  const paths = files.map((f) => f.path);
  const Caret = open ? CaretDown : CaretRight;

  return (
    <div>
      <div
        className="group flex cursor-pointer items-center gap-1 py-0.5 pr-2 text-dk-xs text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-selected)/50"
        style={{ paddingLeft: 8 + depth * 12 }}
        onClick={() => toggleCollapsed(node.path)}
      >
        <Caret size={10} className="shrink-0 text-(--_dk-text-muted)" />
        <FolderIcon size={14} className="shrink-0" />
        <span className="min-w-0 flex-1 truncate">{node.name}</span>
        <FileActions section={section} paths={paths} mutating={mutating} />
      </div>
      {open && (
        <TreeNodes
          nodes={node.children}
          section={section}
          depth={depth + 1}
          collapsed={collapsed}
          toggleCollapsed={toggleCollapsed}
          visible={visible}
        />
      )}
    </div>
  );
}

function FileRow({
  section,
  file,
  depth,
  visible,
}: {
  section: GitSection;
  file: GitFile;
  depth: number;
  visible: string[];
}) {
  const id = gitRowId(section, file.path);
  const selected = useGitStore((s) => s.selected.has(id));
  const allSelected = useGitStore((s) => s.selected);
  const select = useGitStore((s) => s.select);
  const mutating = useGitStore((s) => s.mutating);
  const openFile = useEditorStore((s) => s.openFile);
  const Icon = getFileIcon(fileNameFromPath(file.path));
  const targets = actionTargetPaths(allSelected, section, file.path);
  const letter = gitStatusLabel(file);

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
      style={{ paddingLeft: 8 + depth * 12 }}
      className={`group flex cursor-pointer items-center gap-1 py-0.5 pr-2 text-dk-xs ${
        selected
          ? "bg-(--_dk-ix-bg-selected) text-(--_dk-text-secondary)"
          : "text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-selected)/50"
      }`}
      title={file.orig_path ? `${file.orig_path} → ${file.path}` : file.path}
    >
      <span className={`w-3 shrink-0 font-mono ${gitStatusColor(letter)}`}>
        {letter}
      </span>
      <Icon size={14} className="shrink-0" />
      <span className="min-w-0 flex-1 truncate">{fileNameFromPath(file.path)}</span>
      <FileActions section={section} paths={targets} mutating={mutating} />
    </div>
  );
}

function ChangeGroup({
  section,
  files,
  label,
  headerActionTitle,
  onHeaderAction,
  collapsed,
  toggleCollapsed,
}: {
  section: GitSection;
  files: GitFile[];
  label: string;
  headerActionTitle: string;
  onHeaderAction: () => void;
  collapsed: Set<string>;
  toggleCollapsed: (path: string) => void;
}) {
  const mutating = useGitStore((s) => s.mutating);
  const tree = useMemo(() => buildGitTree(files), [files]);
  const visible = useMemo(
    () => visibleFileIds(tree, section, collapsed, gitRowId),
    [tree, section, collapsed],
  );

  return (
    <FoldCard
      id={`git-${section}`}
      defaultOpen
      className="git-foldcard"
      label={
        <span className="flex min-w-0 flex-1 items-center gap-1">
          <span className={`${FOLDCARD_HEADER_TONE} min-w-0 flex-1 truncate`}>
            {label} ({files.length})
          </span>
          <IconBtn
            title={headerActionTitle}
            disabled={mutating}
            onClick={(e) => {
              e.stopPropagation();
              onHeaderAction();
            }}
          >
            {section === "staged" ? <Minus size={12} /> : <Plus size={12} />}
          </IconBtn>
        </span>
      }
    >
      <TreeNodes
        nodes={tree}
        section={section}
        depth={0}
        collapsed={collapsed}
        toggleCollapsed={toggleCollapsed}
        visible={visible}
      />
    </FoldCard>
  );
}

function CommitRow({
  commit,
  graphWidthPx,
  selected,
  onSelect,
}: {
  commit: GitCommitInfo;
  graphWidthPx: number;
  selected: boolean;
  onSelect: () => void;
}) {
  const tip = [commit.sha, "", commit.author, commit.date, "", commit.subject, commit.body]
    .filter((line, i, arr) => !(line === "" && i === arr.length - 1))
    .join("\n");

  return (
    <Popover
      triggerOn="click"
      width="trigger"
      placement="down-left"
      gap={4}
      className="block"
      panelClassName="rounded"
      trigger={({ toggle }) => (
        <div
          className={`flex cursor-pointer items-center pr-2 text-dk-xs text-(--_dk-text-secondary) transition-[transform,background-color] duration-150 hover:scale-[1.02] hover:bg-(--_dk-ix-bg-selected)/50 hover:text-(--_dk-text-primary) active:scale-[0.98] active:opacity-70 ${
            selected ? "bg-(--_dk-ix-bg-selected)/40" : ""
          }`}
          style={{ height: GIT_GRAPH_ROW_HEIGHT, paddingLeft: graphWidthPx }}
          onClick={() => {
            onSelect();
            toggle();
          }}
        >
          <div className="flex min-w-0 flex-1 gap-1.5">
            <span className="max-w-[45%] shrink-0 truncate text-(--_dk-text-muted)">
              {commit.author}
            </span>
            <span className="min-w-0 truncate">{commit.subject}</span>
          </div>
        </div>
      )}
    >
      <div className="max-h-48 overflow-auto whitespace-pre-wrap p-2 text-dk-xs text-(--_dk-text-secondary)">
        {tip}
      </div>
    </Popover>
  );
}

function CommitsPane({ commits }: { commits: GitCommitInfo[] }) {
  const paneRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [selectedSha, setSelectedSha] = useState<string | null>(null);

  useEffect(() => {
    const el = paneRef.current;
    if (!el) return;
    const measure = () => setWidth(el.clientWidth);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(
    () => layoutGitGraph(commits.map((c) => ({ sha: c.sha, parents: c.parents ?? [] }))),
    [commits],
  );
  const maxLanes = maxLanesForWidth(width);
  const graphWidthPx = graphWidth(layout.laneCount, maxLanes);

  return (
    <div ref={paneRef} className="h-full min-h-0">
      <div className="px-2 py-1 text-dk-xs uppercase tracking-wide text-(--_dk-text-muted)">
        Commits
      </div>
      {commits.length === 0 ? (
        <div className="px-2 py-2 text-(--_dk-text-muted)">No commits yet</div>
      ) : (
        <div className="relative">
          <GitCommitGraph layout={layout} maxLanes={maxLanes} />
          {commits.map((c) => (
            <CommitRow
              key={c.sha}
              commit={c}
              graphWidthPx={graphWidthPx}
              selected={c.sha === selectedSha}
              onSelect={() => setSelectedSha(c.sha)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function readSplit(): number {
  const raw = Number(localStorage.getItem(SPLIT_KEY));
  if (!Number.isFinite(raw)) return SPLIT_DEFAULT;
  return Math.min(0.8, Math.max(0.2, raw));
}

export function GitPanel() {
  const status = useGitStore((s) => s.status);
  const commits = useGitStore((s) => s.commits);
  const message = useGitStore((s) => s.message);
  const loading = useGitStore((s) => s.loading);
  const mutating = useGitStore((s) => s.mutating);
  const error = useGitStore((s) => s.error);
  const setMessage = useGitStore((s) => s.setMessage);
  const refresh = useGitStore((s) => s.refresh);
  const stagePaths = useGitStore((s) => s.stagePaths);
  const unstagePaths = useGitStore((s) => s.unstagePaths);
  const commit = useGitStore((s) => s.commit);
  const pull = useGitStore((s) => s.pull);
  const push = useGitStore((s) => s.push);

  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [split, setSplit] = useState(readSplit);
  const splitRef = useRef(split);
  splitRef.current = split;
  const paneRef = useRef<HTMLDivElement>(null);

  const toggleCollapsed = useCallback((path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  useEffect(() => {
    void refresh({ silent: false });
  }, [refresh]);

  const onSplitterDown = (e: MouseEvent) => {
    e.preventDefault();
    const pane = paneRef.current;
    if (!pane) return;
    const startY = e.clientY;
    const start = splitRef.current;
    const height = pane.getBoundingClientRect().height;
    const onMove = (ev: globalThis.MouseEvent) => {
      if (height <= 0) return;
      const next = Math.min(0.8, Math.max(0.2, start + (ev.clientY - startY) / height));
      splitRef.current = next;
      setSplit(next);
    };
    const onUp = () => {
      localStorage.setItem(SPLIT_KEY, String(splitRef.current));
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const busy = mutating;
  const hasChanges = status.staged.length > 0 || status.changes.length > 0;
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

      {error && (
        <div className="border-b border-(--_dk-red-500) px-2 py-1 text-(--_dk-red-500)">
          {error}
        </div>
      )}

      <div ref={paneRef} className="flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 overflow-auto" style={{ flex: split }}>
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

          <div className="px-1 py-1">
            {!hasChanges ? (
              <div className="px-2 py-3 text-(--_dk-text-muted)">No changes</div>
            ) : (
              <>
                {status.staged.length > 0 && (
                  <ChangeGroup
                    section="staged"
                    files={status.staged}
                    label="Staged Changes"
                    headerActionTitle="Unstage All Changes"
                    onHeaderAction={() => void unstagePaths(status.staged.map((f) => f.path))}
                    collapsed={collapsed}
                    toggleCollapsed={toggleCollapsed}
                  />
                )}
                {status.changes.length > 0 && (
                  <ChangeGroup
                    section="changes"
                    files={status.changes}
                    label="Changes"
                    headerActionTitle="Stage All Changes"
                    onHeaderAction={() => void stagePaths(status.changes.map((f) => f.path))}
                    collapsed={collapsed}
                    toggleCollapsed={toggleCollapsed}
                  />
                )}
              </>
            )}
          </div>
        </div>

        <div
          role="separator"
          aria-orientation="horizontal"
          className="h-1 shrink-0 cursor-ns-resize bg-(--_dk-line) hover:bg-(--_dk-text-muted)"
          onMouseDown={onSplitterDown}
        />

        <div className="min-h-0 overflow-auto" style={{ flex: 1 - split }}>
          <CommitsPane commits={commits} />
        </div>
      </div>
    </div>
  );
}
