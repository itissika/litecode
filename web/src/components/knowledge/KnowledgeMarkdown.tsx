import { createContext, useContext, useMemo, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { citationUrlTransform, classifyWorkspaceLink } from "../../lib/citationRef";
import type { Citation } from "../../lib/knowledge/refDisplay";
import { CitationChip } from "../CitationChip";

const KnowledgeSourceContext = createContext<string | null>(null);

function inlineText(children: ReactNode): string {
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map((child) => inlineText(child)).join("");
  if (children && typeof children === "object" && "props" in children) {
    const props = (children as { props?: { children?: ReactNode } }).props;
    return inlineText(props?.children);
  }
  return "";
}

function WorkspaceRefChip({
  path,
  symbol,
  line,
  lines,
  badLines,
  label,
}: {
  path: string;
  symbol: string | null;
  line: number | null;
  lines: string | null;
  badLines: string | null;
  label?: string;
}) {
  const sourceId = useContext(KnowledgeSourceContext) ?? undefined;
  const citation: Citation = useMemo(
    () => ({ path, symbol, line, lines, badLines, label }),
    [path, symbol, line, lines, badLines, label],
  );
  return <CitationChip citation={citation} sourceId={sourceId} label={label} />;
}

const components: Components = {
  p: ({ children }) => <p>{children}</p>,
  ul: ({ children }) => <ul>{children}</ul>,
  ol: ({ children }) => <ol>{children}</ol>,
  li: ({ children }) => <li>{children}</li>,
  strong: ({ children }) => <strong>{children}</strong>,
  em: ({ children }) => <em>{children}</em>,
  a: ({ href, children }) => {
    const classified = classifyWorkspaceLink(href, { knowledgeBody: true });
    if (classified.action === "chip") {
      const label = inlineText(children).trim() || undefined;
      return (
        <WorkspaceRefChip
          path={classified.path}
          symbol={classified.symbol}
          line={classified.line}
          lines={classified.lines}
          badLines={classified.badLines}
          label={label}
        />
      );
    }
    if (classified.action === "text") return <>{children}</>;
    return (
      <a href={href} target="_blank" rel="noreferrer">
        {children}
      </a>
    );
  },
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }) => {
    const raw = String(children).replace(/\n$/, "");
    if (className || raw.includes("\n")) {
      return (
        <pre className="knowledge-code">
          <code>{raw}</code>
        </pre>
      );
    }
    return <code className="knowledge-inline-code">{raw}</code>;
  },
};

const REMARK_PLUGINS = [remarkGfm];

export function KnowledgeMarkdown({
  sourceId,
  text,
}: {
  sourceId: string;
  text: string;
}) {
  return (
    <KnowledgeSourceContext.Provider value={sourceId}>
      <div className="knowledge-markdown">
        <ReactMarkdown
          remarkPlugins={REMARK_PLUGINS}
          urlTransform={citationUrlTransform}
          components={components}
        >
          {text}
        </ReactMarkdown>
      </div>
    </KnowledgeSourceContext.Provider>
  );
}
