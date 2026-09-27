import { useEffect, useRef, type ReactNode } from "react";
import { ArrowsInSimple, ArrowsOutSimple, Graph } from "@phosphor-icons/react";

import { FoldCard } from "../FoldCard";
import { normalizeKey } from "../../lib/knowledge/markers";
import { openKnowledgeGraphPanel } from "../../lib/knowledge/panel";
import type { KnowledgeIssue, KnowledgeNode } from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeMarkdown, KnowledgeStatusBadge } from "./KnowledgeMarkdown";

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

function KnowledgeCard({ node }: { node: KnowledgeNode }) {
  const expanded = useKnowledgeStore((s) => s.expanded.has(node.id));
  const toggle = useKnowledgeStore((s) => s.toggle);
  const focused = useKnowledgeStore((s) => s.focusedId === node.id);
  const nonce = useKnowledgeStore((s) =>
    s.focusedId === node.id ? s.focusNonce : 0,
  );
  const issues = useKnowledgeStore(
    (s) => s.issuesByNode.get(node.id) ?? NO_ISSUES,
  );
  const errors = issues.filter((issue) => issue.severity === "error").length;
  const warnings = issues.length - errors;
  const key = normalizeKey(node.key);

  return (
    <div
      data-knowledge-id={node.id}
      className={[
        "knowledge-card",
        focused ? "is-focused" : "",
        node.status === "disabled" ? "is-disabled" : "",
        node.status === "pending" ? "is-pending" : "",
        errors > 0 ? "has-error" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {focused ? (
        <span key={nonce} className="knowledge-flash" aria-hidden />
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
          <span className="flex min-w-0 items-center gap-1.5">
            <span className="truncate font-mono text-(--_dk-text-primary)">
              {key}
            </span>
            <span className="shrink-0 text-(--_dk-text-muted)">#{node.id}</span>
            <KnowledgeStatusBadge status={node.status} />
            {errors > 0 ? (
              <span className="knowledge-count is-error" title="错误">
                {errors}
              </span>
            ) : null}
            {warnings > 0 ? (
              <span className="knowledge-count is-warning" title="警告">
                {warnings}
              </span>
            ) : null}
          </span>
        }
      >
        <KnowledgeMarkdown sourceId={node.id} text={node.value} />
        {issues.length > 0 ? (
          <ul className="knowledge-issues">
            {issues.map((issue, index) => (
              <li
                key={`${issue.code}-${issue.ref ?? index}`}
                className={
                  issue.severity === "error" ? "is-error" : "is-warning"
                }
              >
                {issue.message}
              </li>
            ))}
          </ul>
        ) : null}
      </FoldCard>
    </div>
  );
}

export function KnowledgeBrowser() {
  const nodes = useKnowledgeStore((s) => s.nodes);
  const issues = useKnowledgeStore((s) => s.issues);
  const expandAll = useKnowledgeStore((s) => s.expandAll);
  const collapseAll = useKnowledgeStore((s) => s.collapseAll);
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const focusNonce = useKnowledgeStore((s) => s.focusNonce);
  const listRef = useRef<HTMLDivElement>(null);
  const errors = issues.filter((issue) => issue.severity === "error").length;
  const warnings = issues.length - errors;

  useEffect(() => {
    if (focusedId == null) return;
    const el = listRef.current?.querySelector(
      `[data-knowledge-id="${focusedId}"]`,
    );
    el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [focusedId, focusNonce]);

  return (
    <>
      <div className="flex shrink-0 items-center gap-1 border-b border-(--_dk-line) px-2 py-1">
        <span className="text-(--_dk-text-secondary)">知识库</span>
        <span className="text-(--_dk-text-muted)">{nodes.length}</span>
        {errors > 0 ? (
          <span className="knowledge-count is-error" title="错误">
            {errors}
          </span>
        ) : null}
        {warnings > 0 ? (
          <span className="knowledge-count is-warning" title="警告">
            {warnings}
          </span>
        ) : null}
        <span className="flex-1" />
        <IconBtn title="展开全部" onClick={expandAll}>
          <ArrowsOutSimple size={13} />
        </IconBtn>
        <IconBtn title="收起全部" onClick={collapseAll}>
          <ArrowsInSimple size={13} />
        </IconBtn>
        <IconBtn
          title="打开知识图谱"
          onClick={() => {
            openKnowledgeGraphPanel();
          }}
        >
          <Graph size={14} />
        </IconBtn>
      </div>
      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto px-1 py-1">
        {nodes.map((node) => (
          <KnowledgeCard key={node.id} node={node} />
        ))}
      </div>
    </>
  );
}
