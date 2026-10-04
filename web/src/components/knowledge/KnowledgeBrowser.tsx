import { useEffect, useMemo, useRef, useState } from "react";
import { CaretRight, FilePlus, Folder } from "@phosphor-icons/react";

import { FileTreeContextMenu, type FileTreeMenuItem } from "../FileTreeContextMenu";
import { FoldCard } from "../FoldCard";
import { knowledgePreview, normalizeKey } from "../../lib/knowledge/markers";
import { openKnowledgeGraphPanel, revealKnowledgeNode } from "../../lib/knowledge/panel";
import { isVisibleInScrollParent } from "../../lib/knowledge/scrollVisible";
import {
  knowledgeAttentionWarnings,
  knowledgeTitleTone,
} from "../../lib/knowledge/validate";
import type {
  KnowledgeIssue,
  KnowledgeNode,
  KnowledgeUnknownFile,
} from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeAttentionIcon } from "./KnowledgeAttentionIcon";

const NO_ISSUES: KnowledgeIssue[] = [];

type CreateDraft = { kind: "folder" | "node"; parentId: string | null };

type MenuTarget =
  | { kind: "root" }
  | { kind: "folder"; id: string; name: string }
  | { kind: "node"; id: string; name: string };

type DragPayload = { kind: "node" | "folder"; id: string };

const DRAG_MIME = "application/x-litecode-knowledge";

function readDrag(event: React.DragEvent): DragPayload | null {
  const raw = event.dataTransfer.getData(DRAG_MIME);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as DragPayload;
    if (parsed.kind !== "node" && parsed.kind !== "folder") return null;
    if (!parsed.id) return null;
    return parsed;
  } catch {
    return null;
  }
}

function writeDrag(event: React.DragEvent, payload: DragPayload) {
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData(DRAG_MIME, JSON.stringify(payload));
}

function carriesDrag(event: React.DragEvent): boolean {
  return Array.from(event.dataTransfer.types).includes(DRAG_MIME);
}

function createError(result: string): string {
  if (result === "duplicate") return "Already exists";
  if (result === "invalid") return "Invalid";
  return "Couldn't write";
}

function KnowledgeCard({
  node,
  sideFlashPulse,
  onMenu,
  onDragHover,
  onDrop,
}: {
  node: KnowledgeNode;
  sideFlashPulse: number;
  onMenu: (event: React.MouseEvent, target: MenuTarget) => void;
  onDragHover: (folderId: string | null) => void;
  onDrop: (event: React.DragEvent, folderId: string | null) => void;
}) {
  const expanded = useKnowledgeStore((s) => s.expanded.has(node.id));
  const toggle = useKnowledgeStore((s) => s.toggle);
  const focused = useKnowledgeStore((s) => s.focusedId === node.id);
  const flashId = useKnowledgeStore((s) => s.flashId);
  const flashing = flashId === node.id && sideFlashPulse > 0;
  const issues = useKnowledgeStore(
    (s) => s.issuesByNode.get(node.id) ?? NO_ISSUES,
  );
  const titleTone = knowledgeTitleTone(issues, node.status);
  const warnings = knowledgeAttentionWarnings(issues);
  const key = normalizeKey(node.key);
  const summary = node.summary || knowledgePreview(node.value, 1);

  return (
    <div
      data-knowledge-id={node.id}
      className={[
        "knowledge-card",
        focused ? "is-focused" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      draggable
      onDragStart={(event) => {
        event.stopPropagation();
        writeDrag(event, { kind: "node", id: node.id });
      }}
      onDragOver={(event) => {
        if (!carriesDrag(event)) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = "move";
        onDragHover(node.folderId ?? null);
      }}
      onDrop={(event) => onDrop(event, node.folderId ?? null)}
      onContextMenu={(event) =>
        onMenu(event, { kind: "node", id: node.id, name: key })
      }
    >
      {flashing ? (
        <span key={sideFlashPulse} className="knowledge-flash" aria-hidden />
      ) : null}
      <FoldCard
        open={expanded}
        onToggle={(next) => {
          if (next !== expanded) toggle(node.id);
        }}
        edgeBlur={false}
        className="knowledge-foldcard"
        headerClassName="text-dk-xs"
        label={
          <span
            className="knowledge-card-label"
            onClick={() => revealKnowledgeNode(node.id)}
          >
            <span
              className={[
                "knowledge-card-title knowledge-node-title truncate font-mono",
                titleTone ? `is-${titleTone}` : "text-(--_dk-text-primary)",
              ]
                .filter(Boolean)
                .join(" ")}
              title={
                titleTone === "error"
                  ? "Reference problem"
                  : titleTone === "pending"
                    ? "Pending review"
                    : undefined
              }
            >
              {key}
            </span>
            <KnowledgeAttentionIcon warnings={warnings} />
          </span>
        }
      >
        <div
          className="knowledge-side-summary"
          role="button"
          tabIndex={0}
          onClick={() => revealKnowledgeNode(node.id)}
          onKeyDown={(event) => {
            if (event.key !== "Enter" && event.key !== " ") return;
            event.preventDefault();
            revealKnowledgeNode(node.id);
          }}
        >
          {summary}
        </div>
      </FoldCard>
    </div>
  );
}

function KnowledgeUnknownRow({ file }: { file: KnowledgeUnknownFile }) {
  return (
    <div
      className="truncate px-2 py-0.5 font-mono text-dk-xs text-(--_dk-text-muted)"
      title="Unrecognized"
    >
      ?{file.name}
    </div>
  );
}

function belongsToFolder(
  folderId: string | null | undefined,
  parentId: string | null,
  known: Set<string>,
): boolean {
  const resolved =
    folderId != null && known.has(folderId) ? folderId : null;
  return resolved === parentId;
}

function KnowledgeCreateRow({
  draft,
  onDone,
}: {
  draft: CreateDraft;
  onDone: (source: CreateDraft) => void;
}) {
  const createFolder = useKnowledgeStore((s) => s.createFolder);
  const createNode = useKnowledgeStore((s) => s.createNode);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const finished = useRef(false);

  async function submit(raw: string) {
    if (finished.current) return;
    const value = raw.trim();
    if (!value) {
      finished.current = true;
      onDone(draft);
      return;
    }
    const result =
      draft.kind === "folder"
        ? await createFolder(draft.parentId, value)
        : await createNode(draft.parentId, value);
    if (finished.current) return;
    if (result === "ok") {
      finished.current = true;
      onDone(draft);
      return;
    }
    setError(createError(result));
  }

  return (
    <div className="knowledge-create-row">
      {draft.kind === "folder" ? (
        <Folder size={14} weight="fill" aria-hidden />
      ) : (
        <FilePlus size={14} aria-hidden />
      )}
      <input
        autoFocus
        className="knowledge-create-input"
        aria-label={draft.kind === "folder" ? "Folder name" : "Node key"}
        aria-invalid={error != null}
        title={error ?? undefined}
        placeholder={draft.kind === "folder" ? "folder" : "node"}
        value={name}
        onChange={(event) => {
          setName(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void submit(name);
          } else if (event.key === "Escape") {
            event.preventDefault();
            finished.current = true;
            onDone(draft);
          }
        }}
        onBlur={() => {
          void submit(name);
        }}
      />
    </div>
  );
}

function KnowledgeFolderBranch({
  parentId,
  flashId,
  sideFlashPulse,
  draft,
  dropId,
  onCreate,
  onCancelCreate,
  onMenu,
  onDragHover,
  onDrop,
}: {
  parentId: string | null;
  flashId: string | null;
  sideFlashPulse: number;
  draft: CreateDraft | null;
  dropId: string | null;
  onCreate: (kind: CreateDraft["kind"], parentId: string | null) => void;
  onCancelCreate: (source: CreateDraft) => void;
  onMenu: (event: React.MouseEvent, target: MenuTarget) => void;
  onDragHover: (folderId: string | null) => void;
  onDrop: (event: React.DragEvent, folderId: string | null) => void;
}) {
  const folders = useKnowledgeStore((s) => s.folders);
  const nodes = useKnowledgeStore((s) => s.nodes);
  const unknown = useKnowledgeStore((s) => s.unknown);
  const expandedFolders = useKnowledgeStore((s) => s.expandedFolders);
  const toggleFolder = useKnowledgeStore((s) => s.toggleFolder);
  const known = useMemo(
    () => new Set(folders.map((folder) => folder.id)),
    [folders],
  );
  const childFolders = folders.filter((folder) =>
    belongsToFolder(folder.parentId, parentId, known),
  );
  const childNodes = nodes.filter((node) =>
    belongsToFolder(node.folderId, parentId, known),
  );
  const childUnknown = unknown.filter((file) =>
    belongsToFolder(file.folderId, parentId, known),
  );

  return (
    <>
      {draft?.parentId === parentId ? (
        <KnowledgeCreateRow draft={draft} onDone={onCancelCreate} />
      ) : null}
      {childFolders.map((folder) => {
        const open = expandedFolders.has(folder.id);
        return (
          <div key={folder.id} className="knowledge-folder">
            <div
              className={[
                "knowledge-folder-line",
                dropId === folder.id ? "is-drop" : "",
              ]
                .filter(Boolean)
                .join(" ")}
            >
              <button
                type="button"
                className="knowledge-folder-row"
                aria-expanded={open}
                draggable
                onClick={() => toggleFolder(folder.id)}
                onDragStart={(event) => {
                  event.stopPropagation();
                  writeDrag(event, { kind: "folder", id: folder.id });
                }}
                onDragOver={(event) => {
                  if (!carriesDrag(event)) return;
                  event.preventDefault();
                  event.stopPropagation();
                  event.dataTransfer.dropEffect = "move";
                  onDragHover(folder.id);
                }}
                onDrop={(event) => onDrop(event, folder.id)}
                onContextMenu={(event) =>
                  onMenu(event, {
                    kind: "folder",
                    id: folder.id,
                    name: folder.name,
                  })
                }
              >
                <CaretRight
                  size={10}
                  weight="bold"
                  className={
                    open
                      ? "knowledge-folder-caret is-open"
                      : "knowledge-folder-caret"
                  }
                  aria-hidden
                />
                <Folder size={14} weight="fill" aria-hidden />
                <span className="truncate">{folder.name}</span>
              </button>
            </div>
            {open ? (
              <div className="knowledge-folder-children">
                <KnowledgeFolderBranch
                  parentId={folder.id}
                  flashId={flashId}
                  sideFlashPulse={sideFlashPulse}
                  draft={draft}
                  dropId={dropId}
                  onCreate={onCreate}
                  onCancelCreate={onCancelCreate}
                  onMenu={onMenu}
                  onDragHover={onDragHover}
                  onDrop={onDrop}
                />
              </div>
            ) : null}
          </div>
        );
      })}
      {childNodes.map((node) => (
        <KnowledgeCard
          key={node.id}
          node={node}
          sideFlashPulse={flashId === node.id ? sideFlashPulse : 0}
          onMenu={onMenu}
          onDragHover={onDragHover}
          onDrop={onDrop}
        />
      ))}
      {childUnknown.map((file) => (
        <KnowledgeUnknownRow key={file.path} file={file} />
      ))}
    </>
  );
}

export function KnowledgeBrowser() {
  const load = useKnowledgeStore((s) => s.load);
  const loading = useKnowledgeStore((s) => s.loading);
  const error = useKnowledgeStore((s) => s.error);
  const nodeCount = useKnowledgeStore((s) => s.nodes.length);
  const flashId = useKnowledgeStore((s) => s.flashId);
  const flashNonce = useKnowledgeStore((s) => s.flashNonce);
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    const clear = () => setDropId(null);
    window.addEventListener("dragend", clear);
    return () => window.removeEventListener("dragend", clear);
  }, []);
  const listRef = useRef<HTMLDivElement>(null);
  const [sideFlashPulse, setSideFlashPulse] = useState(0);
  const [draft, setDraft] = useState<CreateDraft | null>(null);
  const [menu, setMenu] = useState<
    (MenuTarget & { x: number; y: number }) | null
  >(null);
  const [dropId, setDropId] = useState<string | null>(null);
  const toggleFolder = useKnowledgeStore((s) => s.toggleFolder);
  const expandedFolders = useKnowledgeStore((s) => s.expandedFolders);
  const deleteNode = useKnowledgeStore((s) => s.deleteNode);
  const deleteFolder = useKnowledgeStore((s) => s.deleteFolder);
  const moveNode = useKnowledgeStore((s) => s.moveNode);
  const moveFolder = useKnowledgeStore((s) => s.moveFolder);
  const visibility = useKnowledgeStore((s) => s.visibility);
  const setVisibility = useKnowledgeStore((s) => s.setVisibility);

  function startCreate(kind: CreateDraft["kind"], parentId: string | null) {
    if (parentId && !expandedFolders.has(parentId)) toggleFolder(parentId);
    setDraft({ kind, parentId });
  }

  function openMenu(event: React.MouseEvent, target: MenuTarget) {
    event.preventDefault();
    event.stopPropagation();
    setMenu({ ...target, x: event.clientX, y: event.clientY });
  }

  async function dropOn(event: React.DragEvent, folderId: string | null) {
    event.preventDefault();
    event.stopPropagation();
    setDropId(null);
    const payload = readDrag(event);
    if (!payload) return;
    if (payload.kind === "node") await moveNode(payload.id, folderId);
    else await moveFolder(payload.id, folderId);
  }

  useEffect(() => {
    if (flashId == null) return;
    const container = listRef.current;
    if (!container) return;
    const el = container.querySelector(`[data-knowledge-id="${flashId}"]`);
    if (!el) return;
    const pulse = () => setSideFlashPulse((n) => n + 1);
    if (!isVisibleInScrollParent(container, el)) {
      el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      const timer = window.setTimeout(pulse, 320);
      return () => window.clearTimeout(timer);
    }
    pulse();
  }, [flashId, flashNonce]);

  return (
    <>
      <div className="flex shrink-0 items-center gap-1 border-b border-(--_dk-line) px-2 py-1.5">
        <span className="min-w-0 flex-1 text-dk-2xs uppercase tracking-wide text-(--_dk-text-muted)">
          Knowledge Base
        </span>
        <button
          type="button"
          onClick={() => {
            openKnowledgeGraphPanel();
          }}
          className="rounded px-1 py-0.5 text-dk-2xs text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-selected) hover:text-(--_dk-text-secondary)"
        >
          Open Graph
        </button>
      </div>
      {error ? (
        <p className="shrink-0 px-2 py-1 text-dk-2xs text-(--_dk-text-muted)">
          {error}
        </p>
      ) : null}
      {loading && nodeCount === 0 ? (
        <p className="shrink-0 px-2 py-1 text-dk-2xs text-(--_dk-text-muted)">
          Reading knowledge…
        </p>
      ) : null}
      <div
        ref={listRef}
        className={[
          "knowledge-browser-list min-h-0 flex-1 overflow-y-auto px-1 py-1",
          dropId === "" ? "is-drop" : "",
        ]
          .filter(Boolean)
          .join(" ")}
        onDragOver={(event) => {
          if (!carriesDrag(event)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
          setDropId("");
        }}
        onDrop={(event) => void dropOn(event, null)}
        onContextMenu={(event) => {
          if (event.target !== event.currentTarget) return;
          openMenu(event, { kind: "root" });
        }}
      >
        <KnowledgeFolderBranch
          parentId={null}
          flashId={flashId}
          sideFlashPulse={sideFlashPulse}
          draft={draft}
          dropId={dropId}
          onCreate={startCreate}
          onCancelCreate={(source) => {
            setDraft((current) => (current === source ? null : current));
          }}
          onMenu={openMenu}
          onDragHover={(folderId) => setDropId(folderId ?? "")}
          onDrop={(event, folderId) => void dropOn(event, folderId)}
        />
      </div>
      <div className="knowledge-visibility-bar">
        <button
          type="button"
          role="switch"
          aria-checked={visibility === "public"}
          className={visibility === "public" ? "btn-primary btn-xs" : "btn btn-xs"}
          title={
            visibility === "public"
              ? "knowledge/ at the workspace root, tracked by git"
              : ".litecode/knowledge, not in git"
          }
          onClick={() => {
            void setVisibility(visibility === "public" ? "private" : "public");
          }}
        >
          {visibility}
        </button>
      </div>
      {menu ? (
        <FileTreeContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={menuItems(menu, {
            create: (kind, parentId) => {
              setMenu(null);
              startCreate(kind, parentId);
            },
            remove: () => {
              const target = menu;
              setMenu(null);
              if (target.kind === "node") {
                if (!window.confirm(`Delete node "${target.name}"? Its markdown file will be removed.`)) {
                  return;
                }
                void deleteNode(target.id);
              } else if (target.kind === "folder") {
                if (
                  !window.confirm(
                    `Delete folder "${target.name}"? Nodes inside it will be removed too.`,
                  )
                ) {
                  return;
                }
                void deleteFolder(target.id);
              }
            },
          })}
        />
      ) : null}
    </>
  );
}

function menuItems(
  target: MenuTarget,
  actions: {
    create: (kind: CreateDraft["kind"], parentId: string | null) => void;
    remove: () => void;
  },
): FileTreeMenuItem[] {
  if (target.kind === "node") {
    return [
      {
        id: "delete",
        label: "Delete Node",
        danger: true,
        onClick: actions.remove,
      },
    ];
  }
  const parentId = target.kind === "folder" ? target.id : null;
  const items: FileTreeMenuItem[] = [
    {
      id: "folder",
      label: "New Folder",
      onClick: () => actions.create("folder", parentId),
    },
    {
      id: "node",
      label: "New Node",
      onClick: () => actions.create("node", parentId),
    },
  ];
  if (target.kind === "folder") {
    items.push(
      { id: "sep", separator: true },
      {
        id: "delete",
        label: "Delete Folder",
        danger: true,
        onClick: actions.remove,
      },
    );
  }
  return items;
}
