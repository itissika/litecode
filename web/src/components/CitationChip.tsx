import { X } from "@phosphor-icons/react";
import { useMemo, type MouseEvent } from "react";

import { followCitation } from "../lib/knowledge/panel";
import {
  resolveCitation,
  type Citation,
  type CitationChipModel,
} from "../lib/knowledge/refDisplay";
import { externalPreviewId, useEditorStore } from "../stores/editorStore";
import { useKnowledgeStore } from "../stores/knowledgeStore";
import type { KnowledgeIssue } from "../lib/knowledge/types";

const NO_ISSUES: readonly KnowledgeIssue[] = [];

function stopChipEvent(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
}

function chipClass(model: CitationChipModel, nodrag: boolean): string {
  const names = ["knowledge-token"];
  if (model.file) names.push("is-file");
  if (!model.resolvable) {
    names.push(model.file ? "is-missing" : "is-invalid");
  } else if (model.symbol) {
    names.push("is-symbol");
  }
  if (model.resolvable && model.tone === "drift") names.push("is-drift");
  if (model.resolvable && model.tone === "disabled") names.push("is-disabled");
  if (model.resolvable && model.tone === "warning") names.push("is-warning");
  if (nodrag) names.push("nodrag");
  return names.join(" ");
}

/** The one citation capsule. A resolvable label follows the reference; remove is only for editors. */
export function CitationChip({
  citation,
  sourceId,
  label,
  onRemove,
  nodrag = false,
}: {
  citation: Citation;
  sourceId?: string;
  label?: string;
  onRemove?: () => void;
  nodrag?: boolean;
}) {
  const source = useKnowledgeStore((state) =>
    sourceId ? state.byId.get(sourceId) : undefined,
  );
  const target = useKnowledgeStore((state) => {
    if (citation.kind !== "node") return undefined;
    const key = citation.key.trim();
    return key ? state.byKey.get(key) : undefined;
  });
  const issues = useKnowledgeStore((state) =>
    sourceId ? (state.issuesByNode.get(sourceId) ?? NO_ISSUES) : NO_ISSUES,
  );
  const externalOpen = useEditorStore((state) => {
    if (citation.kind !== "file") return false;
    const id = externalPreviewId(citation.path);
    return state.tabs.some((tab) => tab.external && tab.path === id);
  });
  const model = useMemo(
    () =>
      resolveCitation(citation, {
        source: source ?? null,
        target: target ?? null,
        issues,
        externalOpen,
        label,
      }),
    [citation, source, target, issues, externalOpen, label],
  );

  return (
    <span className={chipClass(model, nodrag)} title={model.title}>
      {model.resolvable ? (
        <button
          type="button"
          className="knowledge-token-label nodrag"
          aria-label={model.label}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            followCitation(citation, sourceId);
          }}
        >
          {model.label}
        </button>
      ) : (
        <span className="knowledge-token-label">{model.label}</span>
      )}
      {onRemove ? (
        <button
          type="button"
          className="knowledge-token-remove nodrag"
          aria-label={`Remove ${model.label}`}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            onRemove();
          }}
        >
          <X size={10} weight="bold" />
        </button>
      ) : null}
    </span>
  );
}
