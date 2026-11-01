import type {
  CSSProperties,
  ElementType,
  HTMLAttributes,
  ReactNode,
} from "react";

import { TreeIndentGuides, treeRowPaddingLeft } from "../TreeIndentGuides";

export type ListRowSize = "tree" | "tree-compact" | "section" | "session";

/**
 * Shared row geometry. Domain rows keep their own content and state classes;
 * this owns the parts every list row repeats: height, padding, indent, guide
 * lines and drop feedback.
 */
const SIZE_CLASS: Record<ListRowSize, string> = {
  /** File tree / knowledge row. */
  tree: "gap-1 px-2 py-0.5 text-sm",
  /** Dense tree row (source control): smaller type, caller supplies the inset. */
  "tree-compact": "gap-1 py-0.5 pr-2 text-dk-xs",
  /** Sidebar folder header. */
  section: "h-8 gap-1.5 rounded px-2 text-[11px]",
  /** Session row inside the sidebar. */
  session: "h-9 gap-2 pr-12",
};

export interface ListRowProps extends HTMLAttributes<HTMLElement> {
  as?: "div" | "button";
  size?: ListRowSize;
  /** Tree depth; computes the left inset when `inset` is omitted. */
  depth?: number;
  /** Explicit left padding, for rows that align to a label instead of a depth. */
  inset?: number;
  /** Draw the ancestor guide lines for `depth`. */
  guides?: boolean;
  /** Drop feedback: insertion line or the target wash. */
  drop?: "before" | "after" | "inside" | null;
  /** `ring` matches folder headers, `outline` matches tree rows. */
  dropTone?: "ring" | "outline";
  /** Keep the shared color transition. Disable for rows that never animated. */
  transition?: boolean;
  children?: ReactNode;
}

export function ListRow({
  as = "div",
  size = "tree",
  depth,
  inset,
  guides = false,
  drop = null,
  dropTone = "ring",
  transition = true,
  className,
  style,
  children,
  ...rest
}: ListRowProps) {
  const Component: ElementType = as;
  const paddingLeft =
    inset ?? (depth !== undefined ? treeRowPaddingLeft(depth) : undefined);
  const rowStyle: CSSProperties = { ...(style ?? {}) };
  if (paddingLeft !== undefined) rowStyle.paddingLeft = paddingLeft;
  if (!transition) rowStyle.transitionProperty = "none";
  const classes = [
    "group relative flex w-full min-w-0 items-center transition-colors",
    SIZE_CLASS[size],
    drop === "inside"
      ? dropTone === "outline"
        ? "outline outline-1 outline-(--_dk-accent-hover)"
        : "bg-(--_dk-ix-bg-selected) shadow-[0_0_0_1px_var(--_dk-ix-ring)]"
      : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <Component
      {...rest}
      {...(as === "button" ? { type: "button" } : {})}
      className={classes}
      style={rowStyle}
    >
      {guides ? <TreeIndentGuides depth={depth ?? 0} /> : null}
      {children}
      {drop === "before" || drop === "after" ? (
        <span
          aria-hidden
          className={`pointer-events-none absolute inset-x-0 z-10 h-0.5 bg-(--_dk-accent-hover) ${
            drop === "before" ? "top-0" : "bottom-0"
          }`}
        />
      ) : null}
    </Component>
  );
}
