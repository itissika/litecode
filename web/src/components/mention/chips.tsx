import { X } from "@phosphor-icons/react";
import { NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";
import { useMemo, type MouseEvent } from "react";

import { humanFileLabel, humanSymbolLabel, normalizeKey } from "../../lib/knowledge/markers";
import { chipForMarker } from "../../lib/knowledge/refDisplay";
import { useEditorStore } from "../../stores/editorStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

function stopChipEvent(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
}

/** Editor chip. The label jumps; only the remove control deletes the mention. */
export function KnowledgeMentionChip({
  node,
  deleteNode,
  sourceIdRef,
}: ReactNodeViewProps & { sourceIdRef: { current: string } }) {
  const id = String(node.attrs.id ?? "");
  const label = normalizeKey(id) || id;
  const key = normalizeKey(id);
  const source = useKnowledgeStore((s) => s.byId.get(sourceIdRef.current));
  const target = useKnowledgeStore((s) => (key ? s.byKey.get(key) : undefined));
  const focusCanvas = useKnowledgeStore((s) => s.focusCanvas);
  const byKey = useMemo(() => {
    if (!key || !target) return new Map<string, NonNullable<typeof target>>();
    return new Map([[key, target]]);
  }, [key, target]);
  const model = source ? chipForMarker(source, id, byKey, label) : null;
  const jumpable = model?.jumpable === true && model.targetId != null;

  return (
    <NodeViewWrapper
      as="span"
      className={
        model?.tone === "error" ? "knowledge-token is-invalid" : "knowledge-token"
      }
    >
      {jumpable ? (
        <button
          type="button"
          className="knowledge-token-label nodrag"
          aria-label={label}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            if (model?.targetId) focusCanvas(model.targetId);
          }}
        >
          {label}
        </button>
      ) : (
        <span className="knowledge-token-label">{label}</span>
      )}
      <button
        type="button"
        className="knowledge-token-remove nodrag"
        aria-label={`移除 ${label}`}
        onMouseDown={stopChipEvent}
        onClick={(event) => {
          stopChipEvent(event);
          deleteNode();
        }}
      >
        <X size={10} weight="bold" />
      </button>
    </NodeViewWrapper>
  );
}

/** Workspace path or symbol capsule. Missing is red, drifted is amber, a symbol is cyan. */
export function FileMentionChip({
  node,
  deleteNode,
  sourceIdRef,
}: ReactNodeViewProps & { sourceIdRef: { current: string } }) {
  const path = String(node.attrs.id ?? "");
  const symbol = String(node.attrs.symbol ?? "").trim();
  const label = symbol ? humanSymbolLabel(path, symbol) : humanFileLabel(path);
  const tone = useKnowledgeStore((state) => {
    const issues = state.issuesByNode.get(sourceIdRef.current) ?? [];
    if (issues.some((issue) => issue.code === "missing_file" && issue.ref === path)) {
      return "missing" as const;
    }
    if (
      symbol &&
      issues.some((issue) => issue.code === "missing_symbol" && issue.ref === symbol)
    ) {
      return "missing" as const;
    }
    if (symbol && issues.some((issue) => issue.code === "symbol_drift" && issue.ref === symbol)) {
      return "drift" as const;
    }
    return symbol ? ("symbol" as const) : ("file" as const);
  });
  const driftMessage = useKnowledgeStore((state) => {
    if (tone !== "drift") return "";
    return (
      state.issuesByNode
        .get(sourceIdRef.current)
        ?.find((issue) => issue.code === "symbol_drift" && issue.ref === symbol)?.message ?? ""
    );
  });

  return (
    <NodeViewWrapper
      as="span"
      title={driftMessage || undefined}
      className={
        tone === "missing"
          ? "knowledge-token is-file is-missing"
          : tone === "drift"
            ? "knowledge-token is-file is-symbol is-drift"
            : tone === "symbol"
              ? "knowledge-token is-file is-symbol"
              : "knowledge-token is-file"
      }
    >
      {tone === "missing" ? (
        <span className="knowledge-token-label">{label}</span>
      ) : (
        <button
          type="button"
          className="knowledge-token-label nodrag"
          aria-label={label}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            void useEditorStore.getState().openFile(path);
          }}
        >
          {label}
        </button>
      )}
      <button
        type="button"
        className="knowledge-token-remove nodrag"
        aria-label={`移除 ${label}`}
        onMouseDown={stopChipEvent}
        onClick={(event) => {
          stopChipEvent(event);
          deleteNode();
        }}
      >
        <X size={10} weight="bold" />
      </button>
    </NodeViewWrapper>
  );
}
