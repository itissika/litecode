import { InfoIcon as PhosphorInfoIcon } from "@phosphor-icons/react";

import type { MetaField } from "./paramMeta";

interface ToolInfoIconProps {
  fields: MetaField[];
}

/**
 * A low-key info glyph that reveals a tool call's non-primary parameters on
 * hover. Pure CSS `group-hover` — no state, no Popover dependency. Renders
 * nothing when there are no extra fields.
 */
export function ToolInfoIcon({ fields }: ToolInfoIconProps) {
  if (fields.length === 0) return null;

  return (
    <span className="group relative inline-flex shrink-0 items-center">
      <PhosphorInfoIcon
        size={12}
        className="cursor-help text-(--_dk-text-muted) hover:brightness-125"
        aria-hidden
      />
      <span
        className="pointer-events-none absolute right-0 top-full z-20 mt-1 hidden min-w-40 max-w-72 flex-col gap-1 rounded-md border border-(--_dk-border-visible) bg-(--_dk-surface-raised) px-2.5 py-2 shadow-lg group-hover:flex"
        role="tooltip"
      >
        {fields.map((f) => (
          <span key={f.key} className="flex flex-col gap-0.5">
            <span className="font-mono text-dk-xs text-(--_dk-text-muted)">
              {f.key}
            </span>
            <span className="whitespace-pre-wrap break-words font-mono text-dk-xs text-(--_dk-text-body)">
              {f.value}
            </span>
          </span>
        ))}
      </span>
    </span>
  );
}
