import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Compact markdown for corner toasts.
 *
 * Deliberately NOT `AgentMarkdown`: that one carries shiki highlighting,
 * mermaid blocks and copy buttons for chat transcripts. A toast only ever
 * needs paragraphs, bold, inline code and links.
 */
export function ToastMarkdown({ text }: { text: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        p: ({ children }: { children?: ReactNode }) => (
          <p className="m-0 leading-snug [&+p]:mt-1">{children}</p>
        ),
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
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
