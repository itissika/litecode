import { useId } from "react";
import { FileText, Graph } from "@phosphor-icons/react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";

import { glassFill } from "../composerCard";

export type KnowledgeSurface = "document" | "canvas";

const MODES = [
  { id: "document", label: "文档", Icon: FileText },
  { id: "canvas", label: "画布", Icon: Graph },
] as const;

/**
 * Frosted capsule that switches the knowledge panel between reading a node
 * and the canvas. Sits over both surfaces at the bottom center; the active
 * pill slides with the same spring the composer's segmented controls use.
 */
export function KnowledgeModeSwitch({
  mode,
  onChange,
}: {
  mode: KnowledgeSurface;
  onChange: (next: KnowledgeSurface) => void;
}) {
  const reduceMotion = useReducedMotion();
  const id = useId();
  const pillId = `knowledge-mode-pill-${id}`;

  return (
    <LayoutGroup id={`knowledge-mode-${id}`}>
      <div
        role="group"
        aria-label="知识面板视图"
        className={`absolute bottom-4 left-1/2 z-50 flex -translate-x-1/2 items-center gap-0.5 rounded-full border border-(--_dk-line) p-0.5 shadow-(--_dk-composer-card-shadow) ${glassFill}`}
      >
        {MODES.map(({ id: next, label, Icon }) => {
          const selected = mode === next;
          return (
            <button
              key={next}
              type="button"
              aria-pressed={selected}
              onClick={() => onChange(next)}
              className={`group relative flex h-7 items-center gap-1.5 rounded-full px-3 text-dk-xs transition-colors duration-150 ${
                selected
                  ? "text-(--_dk-accent-hover)"
                  : "text-(--_dk-text-muted) hover:text-(--_dk-text-secondary)"
              }`}
            >
              {selected ? (
                <motion.span
                  layoutId={pillId}
                  className="absolute inset-0 rounded-full bg-(--_dk-accent-halo)"
                  transition={
                    reduceMotion
                      ? { duration: 0 }
                      : { type: "spring", stiffness: 420, damping: 34 }
                  }
                />
              ) : (
                <span className="pointer-events-none absolute inset-0 rounded-full border border-transparent group-hover:border-(--_dk-line)" />
              )}
              <Icon size={13} weight="regular" className="relative z-10" />
              <span className="relative z-10">{label}</span>
            </button>
          );
        })}
      </div>
    </LayoutGroup>
  );
}
