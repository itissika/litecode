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
import {
  KnowledgeSourceField,
  SaveGlyph,
  type CardSaveMark,
} from "./KnowledgeSourceField";

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
  const [saveMark, setSaveMark] = useState<CardSaveMark | null>(null);
  const summaryDraftRef = useRef("");
  const bodyDraftRef = useRef("");
  const summaryDirtyRef = useRef(false);
  const bodyDirtyRef = useRef(false);
  const summaryGen = useRef(0);
  const bodyGen = useRef(0);
  const inflight = useRef(0);
  const hideToken = useRef(0);
  const hideTimer = useRef<number | null>(null);
  const mounted = useRef(true);
  const nodeIdRef = useRef(data.nodeId);
  nodeIdRef.current = data.nodeId;

  useEffect(() => {
    if (!node || summaryDirty) return;
    summaryDraftRef.current = node.summary;
    setSummaryDraft(node.summary);
  }, [node, summaryDirty]);

  useEffect(() => {
    if (!node || bodyDirty) return;
    const next = node.value.replace(/\s+$/, "");
    bodyDraftRef.current = next;
    setBodyDraft(next);
  }, [node, bodyDirty]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (hideTimer.current != null) window.clearTimeout(hideTimer.current);
      const state = useKnowledgeStore.getState();
      const current = state.byId.get(nodeIdRef.current);
      if (!current) return;
      if (summaryDirtyRef.current) {
        const text = summaryDraftRef.current.trim();
        if (text !== current.summary) {
          void state.saveNode(current.id, { summary: text });
        }
      }
      if (bodyDirtyRef.current) {
        const text = bodyDraftRef.current;
        if (text !== current.value.replace(/\s+$/, "")) {
          void state.saveNode(current.id, {
            value: text,
            relations: extractMarkers(text),
          });
        }
      }
    };
  }, []);

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

  function clearSavedLater() {
    hideToken.current += 1;
    if (hideTimer.current != null) {
      window.clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
  }

  function markDirty(field: "summary" | "body") {
    if (field === "summary") {
      summaryGen.current += 1;
      summaryDirtyRef.current = true;
      setSummaryDirty(true);
    } else {
      bodyGen.current += 1;
      bodyDirtyRef.current = true;
      setBodyDirty(true);
    }
    clearSavedLater();
    setSaveMark("dirty");
  }

  function settleMark() {
    if (inflight.current > 0) {
      setSaveMark("saving");
      return;
    }
    if (summaryDirtyRef.current || bodyDirtyRef.current) {
      setSaveMark("dirty");
      return;
    }
    const token = hideToken.current;
    setSaveMark("saved");
    hideTimer.current = window.setTimeout(() => {
      if (hideToken.current !== token) return;
      if (summaryDirtyRef.current || bodyDirtyRef.current || inflight.current > 0) {
        return;
      }
      setSaveMark(null);
    }, 1000);
  }

  function releaseIdle() {
    if (inflight.current > 0) {
      setSaveMark("saving");
      return;
    }
    if (summaryDirtyRef.current || bodyDirtyRef.current) {
      setSaveMark("dirty");
      return;
    }
    clearSavedLater();
    setSaveMark(null);
  }

  async function commitSummary() {
    const gen = summaryGen.current;
    const text = summaryDraftRef.current.trim();
    const current = useKnowledgeStore.getState().byId.get(nodeIdRef.current);
    if (!current) return;
    if (text === current.summary) {
      summaryDirtyRef.current = false;
      setSummaryDirty(false);
      releaseIdle();
      return;
    }
    inflight.current += 1;
    clearSavedLater();
    setSaveMark("saving");
    const ok = await saveNode(current.id, { summary: text });
    inflight.current = Math.max(0, inflight.current - 1);
    if (!mounted.current) return;
    if (summaryGen.current !== gen) {
      if (inflight.current === 0) setSaveMark("dirty");
      return;
    }
    if (!ok) {
      setSaveMark(inflight.current > 0 ? "saving" : "dirty");
      return;
    }
    summaryDirtyRef.current = false;
    setSummaryDirty(false);
    settleMark();
  }

  async function commitBody() {
    const gen = bodyGen.current;
    const text = bodyDraftRef.current;
    const current = useKnowledgeStore.getState().byId.get(nodeIdRef.current);
    if (!current) return;
    if (text === current.value.replace(/\s+$/, "")) {
      bodyDirtyRef.current = false;
      setBodyDirty(false);
      releaseIdle();
      return;
    }
    inflight.current += 1;
    clearSavedLater();
    setSaveMark("saving");
    const ok = await saveNode(current.id, {
      value: text,
      relations: extractMarkers(text),
    });
    inflight.current = Math.max(0, inflight.current - 1);
    if (!mounted.current) return;
    if (bodyGen.current !== gen) {
      if (inflight.current === 0) setSaveMark("dirty");
      return;
    }
    if (!ok) {
      setSaveMark(inflight.current > 0 ? "saving" : "dirty");
      return;
    }
    bodyDirtyRef.current = false;
    setBodyDirty(false);
    settleMark();
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
          <SaveGlyph mark={saveMark} />
          <span className="knowledge-flow-title-fill" />
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
                summaryDraftRef.current = next;
                setSummaryDraft(next);
                markDirty("summary");
              }}
              onBlur={() => {
                void commitSummary();
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
                bodyDraftRef.current = next;
                setBodyDraft(next);
                markDirty("body");
              }}
              onBlur={() => {
                void commitBody();
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
            <ArrowsOutSimple size={12} weight="bold" aria-hidden />
          </NodeResizeControl>
        ) : null}
      </div>
      <Handle type="source" position={Position.Right} isConnectable={false} />
    </>
  );
}
