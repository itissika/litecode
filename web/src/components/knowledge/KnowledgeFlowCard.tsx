import { useEffect, useRef, useState } from "react";
import { ArrowsOutSimple, CaretDown } from "@phosphor-icons/react";
import {
  Handle,
  NodeResizeControl,
  Position,
  type Node,
  type NodeProps,
} from "@xyflow/react";

import {
  KNOWLEDGE_NODE_MAX_HEIGHT,
  KNOWLEDGE_NODE_MAX_WIDTH,
  KNOWLEDGE_NODE_MIN_HEIGHT,
  KNOWLEDGE_NODE_MIN_WIDTH,
} from "../../lib/knowledge/layoutGraph";
import { extractMarkers, knowledgePreview, normalizeKey } from "../../lib/knowledge/markers";
import { knowledgeTitleTone } from "../../lib/knowledge/validate";
import type { KnowledgeIssue } from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeSourceField } from "./KnowledgeSourceField";

const NO_ISSUES: KnowledgeIssue[] = [];

type KnowledgeNodeData = { nodeId: string };

export function KnowledgeFlowCard({
  data,
}: NodeProps<Node<KnowledgeNodeData, "knowledge">>) {
  const node = useKnowledgeStore((s) => s.byId.get(data.nodeId));
  const nodes = useKnowledgeStore((s) => s.nodes);
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const focusNode = useKnowledgeStore((s) =>
    focusedId == null ? undefined : s.byId.get(focusedId),
  );
  const graphOpen = useKnowledgeStore((s) => s.graphExpanded.has(data.nodeId));
  const toggleGraph = useKnowledgeStore((s) => s.toggleGraph);
  const saveNode = useKnowledgeStore((s) => s.saveNode);
  const renameNode = useKnowledgeStore((s) => s.renameNode);
  const issues = useKnowledgeStore(
    (s) => s.issuesByNode.get(data.nodeId) ?? NO_ISSUES,
  );
  const [editingKey, setEditingKey] = useState(false);
  const [keyDraft, setKeyDraft] = useState("");
  const [keyError, setKeyError] = useState<string | null>(null);
  const [summaryDraft, setSummaryDraft] = useState("");
  const [bodyDraft, setBodyDraft] = useState("");
  const [summaryDirty, setSummaryDirty] = useState(false);
  const [bodyDirty, setBodyDirty] = useState(false);
  const summaryTimer = useRef<number | null>(null);
  const bodyTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!node || summaryDirty) return;
    setSummaryDraft(node.summary);
  }, [node, summaryDirty]);

  useEffect(() => {
    if (!node || bodyDirty) return;
    setBodyDraft(node.value.replace(/\s+$/, ""));
  }, [node, bodyDirty]);

  if (!node) return null;

  const titleTone = knowledgeTitleTone(issues, node.status);
  const focused = focusedId === node.id;
  const ownKey = normalizeKey(node.key);
  const focusKey = focusNode ? normalizeKey(focusNode.key) : "";
  let dimmed = false;
  if (focusedId != null && !focused && focusNode) {
    const linked =
      focusNode.relations.includes(ownKey) || node.relations.includes(focusKey);
    dimmed = !linked;
  }
  const summary = node.summary || knowledgePreview(node.value, 1);
  const refCandidates = nodes
    .map((item) => normalizeKey(item.key))
    .filter((key) => key && key !== ownKey);
  const enabled = node.status === "enabled";

  async function commitKey() {
    const result = await renameNode(node!.id, keyDraft);
    if (result === "duplicate") {
      setKeyError("已存在");
      return;
    }
    if (result === "invalid") {
      setKeyError("不合法");
      return;
    }
    setKeyError(null);
    setEditingKey(false);
  }

  return (
    <>
      <Handle type="target" position={Position.Left} isConnectable={false} />
      <div
        className={[
          "knowledge-flow-card",
          graphOpen ? "is-expanded" : "",
          focused ? "is-focused" : "",
          dimmed ? "is-dimmed" : "",
        ]
          .filter(Boolean)
          .join(" ")}
      >
        {node.status === "disabled" && !focused ? (
          <div className="knowledge-flow-disabled-veil" aria-hidden />
        ) : null}
        <div className="knowledge-flow-title">
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            aria-label={enabled ? "禁用" : "启用"}
            className={
              enabled
                ? "knowledge-enable nodrag nowheel is-on"
                : "knowledge-enable nodrag nowheel"
            }
            onClick={(event) => {
              event.stopPropagation();
              void saveNode(node.id, {
                status: enabled ? "disabled" : "enabled",
              });
            }}
          />
          {editingKey ? (
            <input
              className="knowledge-key-input nodrag nowheel"
              value={keyDraft}
              aria-invalid={keyError != null}
              title={keyError ?? undefined}
              autoFocus
              onChange={(event) => {
                setKeyDraft(event.target.value);
                setKeyError(null);
              }}
              onBlur={() => {
                void commitKey();
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void commitKey();
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  setEditingKey(false);
                  setKeyError(null);
                }
              }}
            />
          ) : (
            <button
              type="button"
              className={[
                "knowledge-card-title knowledge-node-title knowledge-key-button nowheel truncate font-mono",
                titleTone ? `is-${titleTone}` : "",
              ]
                .filter(Boolean)
                .join(" ")}
              onClick={(event) => {
                event.stopPropagation();
                setKeyDraft(ownKey);
                setKeyError(null);
                setEditingKey(true);
              }}
            >
              {ownKey}
            </button>
          )}
          <button
            type="button"
            className={
              graphOpen
                ? "knowledge-flow-caret nodrag nowheel is-open"
                : "knowledge-flow-caret nodrag nowheel"
            }
            aria-expanded={graphOpen}
            aria-label={graphOpen ? "收起" : "展开"}
            onClick={(event) => {
              event.stopPropagation();
              toggleGraph(node.id);
            }}
          >
            <CaretDown size={12} weight="bold" />
          </button>
        </div>
        {graphOpen ? (
          <div className="knowledge-flow-editors">
            <KnowledgeSourceField
              label="摘要"
              rows={2}
              singleLine
              value={summaryDraft}
              candidates={[]}
              onChange={(next) => {
                setSummaryDraft(next);
                setSummaryDirty(true);
                if (summaryTimer.current != null) {
                  window.clearTimeout(summaryTimer.current);
                }
                summaryTimer.current = window.setTimeout(() => {
                  void saveNode(node.id, { summary: next.trim() });
                }, 400);
              }}
              onBlur={() => {
                if (summaryTimer.current != null) {
                  window.clearTimeout(summaryTimer.current);
                }
                void saveNode(node.id, { summary: summaryDraft.trim() });
                setSummaryDirty(false);
              }}
            />
            <KnowledgeSourceField
              label="正文"
              kind="body"
              sourceId={node.id}
              className="is-fill"
              rows={5}
              value={bodyDraft}
              candidates={refCandidates}
              onChange={(next) => {
                setBodyDraft(next);
                setBodyDirty(true);
                if (bodyTimer.current != null) window.clearTimeout(bodyTimer.current);
                bodyTimer.current = window.setTimeout(() => {
                  void saveNode(node.id, {
                    value: next,
                    relations: extractMarkers(next),
                  });
                }, 400);
              }}
              onBlur={() => {
                if (bodyTimer.current != null) window.clearTimeout(bodyTimer.current);
                void saveNode(node.id, {
                  value: bodyDraft,
                  relations: extractMarkers(bodyDraft),
                });
                setBodyDirty(false);
              }}
            />
          </div>
        ) : (
          <div className="knowledge-flow-summary">{summary}</div>
        )}
        {graphOpen ? (
          <NodeResizeControl
            position="bottom-right"
            minWidth={KNOWLEDGE_NODE_MIN_WIDTH}
            minHeight={KNOWLEDGE_NODE_MIN_HEIGHT}
            maxWidth={KNOWLEDGE_NODE_MAX_WIDTH}
            maxHeight={KNOWLEDGE_NODE_MAX_HEIGHT}
            className="knowledge-flow-resize-handle"
          >
            <ArrowsOutSimple size={10} weight="bold" aria-hidden />
          </NodeResizeControl>
        ) : null}
      </div>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </>
  );
}
