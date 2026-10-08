import { relationStripChips } from "../../lib/knowledge/refDisplay";
import type { KnowledgeNode } from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { CitationChip } from "../CitationChip";

export function KnowledgeRelationStrip({ node }: { node: KnowledgeNode }) {
  const byId = useKnowledgeStore((state) => state.byId);
  const chips = relationStripChips(node, byId);
  if (chips.length === 0) return null;

  return (
    <div className="knowledge-relations-strip" aria-label="Registered relations">
      {chips.map((chip) => (
        <CitationChip
          key={chip.targetId ?? chip.key}
          citation={{ kind: "node", key: chip.key }}
          sourceId={node.id}
          label={chip.label}
        />
      ))}
    </div>
  );
}
