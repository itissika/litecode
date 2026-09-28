import { bodyMarkerKeys, relationStripChips } from "../../lib/knowledge/refDisplay";
import type { KnowledgeNode } from "../../lib/knowledge/types";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeRefChip } from "./KnowledgeRefChip";

export function KnowledgeRelationStrip({ node }: { node: KnowledgeNode }) {
  const byId = useKnowledgeStore((s) => s.byId);
  const focusCanvas = useKnowledgeStore((s) => s.focusCanvas);
  const bodyKeys = bodyMarkerKeys(node.value);
  const chips = relationStripChips(node, byId, bodyKeys);
  if (chips.length === 0) return null;

  return (
    <div className="knowledge-relations-strip" aria-label="Registered relations">
      {chips.map((chip) => (
        <KnowledgeRefChip
          key={chip.targetId ?? chip.key}
          model={chip}
          silent
          onActivate={() => {
            if (chip.targetId != null) focusCanvas(chip.targetId);
          }}
        />
      ))}
    </div>
  );
}
