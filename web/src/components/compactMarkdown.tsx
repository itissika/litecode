import type { ReactNode } from "react";
import type { Components } from "react-markdown";

/**
 * Inline markdown mappings shared by compact renderers (toasts, ask cards).
 * Each caller adds its own block-level mappings (p, lists, pre) on top.
 */
export const compactMarkdownComponents: Components = {
  strong: ({ children }: { children?: ReactNode }) => (
    <strong className="font-semibold text-(--_dk-text-primary)">
      {children}
    </strong>
  ),
  em: ({ children }: { children?: ReactNode }) => (
    <em className="italic">{children}</em>
  ),
  code: ({ children }: { children?: ReactNode }) => (
    <code className="rounded-sm bg-(--_dk-side) px-1 py-px font-mono text-[12px] text-(--_dk-text-primary)">
      {children}
    </code>
  ),
  a: ({ href, children }: { href?: string; children?: ReactNode }) => (
    <a
      href={href}
      className="text-(--_dk-accent-hover) underline"
      target="_blank"
      rel="noreferrer"
    >
      {children}
    </a>
  ),
};
