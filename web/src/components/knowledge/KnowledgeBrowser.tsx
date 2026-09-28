import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { CaretRight, Folder, Graph, WarningCircle } from "@phosphor-icons/react";

import { FoldCard } from "../FoldCard";
import { normalizeKey } from "../../lib/knowledge/markers";
import { openKnowledgeGraphPanel } from "../../lib/knowledge/panel";
import { isVisibleInScrollParent } from "../../lib/knowledge/scrollVisible";
import { bodyMarkerKeys, relationStripChips } from "../../lib/knowledge/refDisplay";
import { knowledgeListAlert } from "../../lib/knowledge/validate";
import type { KnowledgeIssue, KnowledgeNode } from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeMarkdown } from "./KnowledgeMarkdown";
import { KnowledgeRelationStrip } from "./KnowledgeRelationStrip";

const NO_ISSUES: KnowledgeIssue[] = [];

function IconBtn({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      className="rounded p-0.5 text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-selected) hover:text-(--_dk-text-secondary)"
    >
      {children}
    </button>
  );
}

function KnowledgeCard({
  node,
  sideFlashPulse,
}: {
  node: KnowledgeNode;
  sideFlashPulse: number;
}) {
  const expanded = useKnowledgeStore((s) => s.expanded.has(node.id));
  const toggle = useKnowledgeStore((s) => s.toggle);
  const focused = useKnowledgeStore((s) => s.focusedId === node.id);
  const flashId = useKnowledgeStore((s) => s.flashId);
  const focusCanvas = useKnowledgeStore((s) => s.focusCanvas);
  const flashing = flashId === node.id && sideFlashPulse > 0;
  const issues = useKnowledgeStore(
    (s) => s.issuesByNode.get(node.id) ?? NO_ISSUES,
  );
  const byId = useKnowledgeStore((s) => s.byId);
  const showRelationStrip = useMemo(
    () =>
      relationStripChips(node, byId, bodyMarkerKeys(node.value)).length > 0,
    [node, byId],
  );
  const key = normalizeKey(node.key);
  const listAlert = knowledgeListAlert(issues, node.status);
  const disabled = node.status === "disabled";

  return (
    <div
      data-knowledge-id={node.id}
      className={["knowledge-card", focused ? "is-focused" : ""]
        .filter(Boolean)
        .join(" ")}
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
          <span className="flex min-w-0 items-center gap-1">
            {listAlert ? (
              <WarningCircle
                size={12}
                weight="fill"
                className={
                  listAlert === "red"
                    ? "knowledge-list-alert is-red"
                    : "knowledge-list-alert is-amber"
                }
                aria-label={
                  listAlert === "red"
                    ? "Reference problem"
                    : "Pending review"
                }
              />
            ) : null}
            <span
              className={[
                "knowledge-card-title knowledge-node-title truncate font-mono",
                disabled ? "is-disabled" : "text-(--_dk-text-primary)",
              ]
                .filter(Boolean)
                .join(" ")}
            >
              {key}
            </span>
          </span>
        }
      >
        <div
          className="knowledge-card-body"
          role="button"
          tabIndex={0}
          onClick={() => focusCanvas(node.id)}
          onKeyDown={(event) => {
            if (event.key !== "Enter" && event.key !== " ") return;
            event.preventDefault();
            focusCanvas(node.id);
          }}
        >
          <KnowledgeMarkdown sourceId={node.id} text={node.value} />
          {showRelationStrip ? (
            <>
              <div className="knowledge-relations-divider" aria-hidden />
              <KnowledgeRelationStrip node={node} />
            </>
          ) : null}
        </div>
      </FoldCard>
    </div>
  );
}

function belongsToFolder(
  folderId: number | null | undefined,
  parentId: number | null,
  known: Set<number>,
): boolean {
  const resolved =
    folderId != null && known.has(folderId) ? folderId : null;
  return resolved === parentId;
}

function KnowledgeFolderBranch({
  parentId,
  flashId,
  sideFlashPulse,
}: {
  parentId: number | null;
  flashId: number | null;
  sideFlashPulse: number;
}) {
  const folders = useKnowledgeStore((s) => s.folders);
  const nodes = useKnowledgeStore((s) => s.nodes);
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

  return (
    <>
      {childFolders.map((folder) => {
        const open = expandedFolders.has(folder.id);
        return (
          <div key={folder.id} className="knowledge-folder">
            <button
              type="button"
              className="knowledge-folder-row"
              aria-expanded={open}
              onClick={() => toggleFolder(folder.id)}
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
            {open ? (
              <div className="knowledge-folder-children">
                <KnowledgeFolderBranch
                  parentId={folder.id}
                  flashId={flashId}
                  sideFlashPulse={sideFlashPulse}
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
        />
      ))}
    </>
  );
}

export function KnowledgeBrowser() {
  const flashId = useKnowledgeStore((s) => s.flashId);
  const flashNonce = useKnowledgeStore((s) => s.flashNonce);
  const listRef = useRef<HTMLDivElement>(null);
  const [sideFlashPulse, setSideFlashPulse] = useState(0);

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
        <IconBtn
          title="Open knowledge graph"
          onClick={() => {
            openKnowledgeGraphPanel();
          }}
        >
          <Graph size={14} />
        </IconBtn>
      </div>
      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-1 py-1">
        <KnowledgeFolderBranch
          parentId={null}
          flashId={flashId}
          sideFlashPulse={sideFlashPulse}
        />
      </div>
    </>
  );
}
