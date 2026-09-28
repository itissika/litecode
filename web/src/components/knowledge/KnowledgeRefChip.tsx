import { Tag, X } from "@phosphor-icons/react";

import type { RefChipModel, RefChipTone } from "../../lib/knowledge/refDisplay";

function toneClass(tone: RefChipTone): string {
  if (tone === "error") return "knowledge-ref is-error";
  if (tone === "disabled") return "knowledge-ref is-disabled-target";
  if (tone === "warning") return "knowledge-ref is-warning";
  return "knowledge-ref";
}

export function KnowledgeRefChip({
  model,
  onActivate,
  silent,
}: {
  model: RefChipModel;
  onActivate?: () => void;
  /** Omit hover titles (relation strip). */
  silent?: boolean;
}) {
  const Icon = model.jumpable ? Tag : X;
  const title =
    silent || !model.jumpable
      ? undefined
      : model.tone === "normal"
        ? model.key
        : undefined;

  return (
    <button
      type="button"
      className={toneClass(model.tone)}
      title={title}
      aria-label={model.key}
      aria-invalid={model.tone === "error" ? true : undefined}
      disabled={!model.jumpable}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (model.jumpable) onActivate?.();
      }}
    >
      <Icon size={10} weight="fill" className="knowledge-ref-icon" aria-hidden />
      {model.key}
    </button>
  );
}
