import { createContext, useContext, useMemo, type ReactNode } from "react";
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
} from "react-markdown";
import remarkGfm from "remark-gfm";

import {
  parseKnowledgeRef,
  parseLocationRef,
  remarkKnowledgeRef,
} from "../../lib/knowledge/remarkKnowledgeRef";
import { CitationChip } from "../CitationChip";

const KnowledgeSourceContext = createContext<string | null>(null);

function knowledgeUrlTransform(url: string): string {
  if (parseKnowledgeRef(url) || parseLocationRef(url)) return url;
  return defaultUrlTransform(url);
}

function inlineText(children: ReactNode): string {
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map((child) => inlineText(child)).join("");
  if (children && typeof children === "object" && "props" in children) {
    const props = (children as { props?: { children?: ReactNode } }).props;
    return inlineText(props?.children);
  }
  return "";
}

function BodyRefChip({ marker, label }: { marker: string; label: string }) {
  const sourceId = useContext(KnowledgeSourceContext);
  if (!sourceId) return <span>{marker}</span>;
  return (
    <CitationChip
      citation={{ kind: "node", key: marker }}
      sourceId={sourceId}
      label={label}
    />
  );
}

function FileRefChip({
  path,
  symbol,
  line,
  lines,
}: {
  path: string;
  symbol: string | null;
  line: number | null;
  lines: string | null;
}) {
  const sourceId = useContext(KnowledgeSourceContext) ?? undefined;
  const citation = useMemo(
    () => ({ kind: "file" as const, path, symbol, line, lines }),
    [path, symbol, line, lines],
  );
  return (
    <CitationChip
      citation={citation}
      sourceId={sourceId}
    />
  );
}

const components: Components = {
  p: ({ children }) => <p>{children}</p>,
  ul: ({ children }) => <ul>{children}</ul>,
  ol: ({ children }) => <ol>{children}</ol>,
  li: ({ children }) => <li>{children}</li>,
  strong: ({ children }) => <strong>{children}</strong>,
  em: ({ children }) => <em>{children}</em>,
  a: ({ href, children }) => {
    const key = parseKnowledgeRef(href);
    if (key) return <BodyRefChip marker={key} label={inlineText(children) || key} />;
    const located = parseLocationRef(href);
    if (located) {
      return (
        <FileRefChip
          path={located.path}
          symbol={located.symbol}
          line={located.line}
          lines={located.lines ?? null}
        />
      );
    }
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
          remarkPlugins={[remarkGfm, remarkKnowledgeRef]}
          urlTransform={knowledgeUrlTransform}
          components={components}
        >
          {text}
        </ReactMarkdown>
      </div>
    </KnowledgeSourceContext.Provider>
  );
}
