import type { ReactElement } from "react";
import { FOLDCARD_HEADER_TONE } from "./FoldCard";

interface CategoryCountProps {
  icon: ReactElement;
  count: number;
  /** Category noun shown before the count, e.g. "reasoning", "bash", "tool". */
  noun: string;
}

function displayNoun(noun: string, count: number): string {
  if (noun === "tool" && count !== 1) return "tools";
  return noun;
}

/** Process-group header badge: icon + category label × count. */
export function CategoryCount({ icon, count, noun }: CategoryCountProps) {
  if (count <= 0) return null;
  return (
    <span className={`${FOLDCARD_HEADER_TONE} inline-flex items-center gap-1`}>
      <span className="inline-flex shrink-0">{icon}</span>
      <span className="text-dk-sm text-(--_dk-text-secondary)">
        {displayNoun(noun, count)}
      </span>
      <span className="inline-flex items-baseline font-mono text-dk-sm tabular-nums text-(--_dk-text-secondary)">
        ×{count}
      </span>
    </span>
  );
}
