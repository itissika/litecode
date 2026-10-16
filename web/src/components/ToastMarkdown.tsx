import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { compactMarkdownComponents } from "./compactMarkdown";

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
        ...compactMarkdownComponents,
        p: ({ children }: { children?: ReactNode }) => (
          <p className="m-0 leading-snug [&+p]:mt-1">{children}</p>
        ),
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
