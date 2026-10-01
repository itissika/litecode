import { Copy, Check } from "@phosphor-icons/react";
import { memo, useEffect, useMemo, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import {
  citationUrlTransform,
  isHttpCitation,
} from "../lib/citationRef";
import { normalizeKey } from "../lib/knowledge/markers";
import {
  parseKnowledgeRef,
  remarkKnowledgeRef,
} from "../lib/knowledge/remarkKnowledgeRef";
import { useKnowledgeStore } from "../stores/knowledgeStore";
import {
  getMarkdownHighlighter,
  isSupportedHighlightLang,
  SHIKI_THEME_DARK,
  SHIKI_THEME_LIGHT,
  normalizeLang,
} from "../lib/shiki";
import { isMermaidLang } from "../lib/mermaid";
import { useStreamingBuffer } from "../lib/streamingBuffer";
import { FileCitationChip, WebCitationChip } from "./CitationChip";
import { MermaidBlock } from "./MermaidBlock";

const GENERIC_LANGS = new Set(["", "text", "txt", "plain", "plaintext"]);

function MarkdownCodeBlock({ code, lang }: { code: string; lang: string }) {
  const [html, setHtml] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const normalizedLang = useMemo(() => normalizeLang(lang), [lang]);
  const displayLang = lang || "text";
  const skipHighlight =
    GENERIC_LANGS.has(normalizedLang) ||
    !isSupportedHighlightLang(normalizedLang);

  useEffect(() => {
    if (skipHighlight) {
      setHtml(null);
      return;
    }

    let cancelled = false;

    void (async () => {
      try {
        const highlighter = await getMarkdownHighlighter();
        const highlighted = highlighter.codeToHtml(code, {
          lang: normalizedLang,
          themes: { light: SHIKI_THEME_LIGHT, dark: SHIKI_THEME_DARK },
        });
        if (!cancelled) setHtml(highlighted);
      } catch {
        if (!cancelled) setHtml(null);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [code, normalizedLang, skipHighlight]);

  const copyCode = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      /* clipboard unavailable */
    }
  };

  return (
    <div className="agent-code-block mb-2 last:mb-0">
      <div className="agent-code-block-header">
        <span className="agent-code-block-lang">{displayLang}</span>
        <button
          type="button"
          onClick={() => void copyCode()}
          className="agent-code-block-copy"
          aria-label={copied ? "Copied" : "Copy code"}
        >
          {copied ? (
            <Check size={12} weight="bold" aria-hidden />
          ) : (
            <Copy size={12} aria-hidden />
          )}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      {html ? (
        <div
          className="agent-code-block-body shiki-host"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="agent-code-block-fallback">
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}

function linkLabel(children: ReactNode): string {
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map((child) => linkLabel(child)).join("");
  return "";
}

function NodeCitationChip({ id, label }: { id: string; label: string }) {
  const key = normalizeKey(id);
  const target = useKnowledgeStore((state) => (key ? state.byKey.get(key) : undefined));
  const focusCanvas = useKnowledgeStore((state) => state.focusCanvas);
  const shown = label || key;
  return (
    <span className={target ? "knowledge-token" : "knowledge-token is-invalid"}>
      {target ? (
        <button
          type="button"
          className="knowledge-token-label"
          aria-label={shown}
          onMouseDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            focusCanvas(target.id);
          }}
        >
          {shown}
        </button>
      ) : (
        <span className="knowledge-token-label">{shown}</span>
      )}
    </span>
  );
}

interface AgentMarkdownProps {
  text: string;
  streaming?: boolean;
  /** Assistant prose only. Other callers keep ordinary links. */
  citations?: boolean;
  /** Sit in the surrounding line. Used when a mention capsule shares the sentence. */
  inline?: boolean;
}

export const AgentMarkdown = memo(function AgentMarkdown({
  text,
  streaming = false,
  citations = false,
  inline = false,
}: AgentMarkdownProps) {
  const displayText = useStreamingBuffer(text, streaming);
  const remarkPlugins = useMemo(
    () => (citations ? [remarkGfm, remarkKnowledgeRef] : [remarkGfm]),
    [citations],
  );
  const transformUrl = useMemo(
    () => (value: string) => {
      if (citations && parseKnowledgeRef(value)) return value;
      return citationUrlTransform(value);
    },
    [citations],
  );
  const components = useMemo(
    () => ({
      p: ({ children }: { children?: ReactNode }) =>
        inline ? (
          <span className="leading-relaxed">{children}</span>
        ) : (
          <p className="mb-2.5 last:mb-0 leading-relaxed">{children}</p>
        ),
      ul: ({ children }: { children?: ReactNode }) => (
        <ul className="mb-2 list-disc pl-5 last:mb-0">{children}</ul>
      ),
      ol: ({ children }: { children?: ReactNode }) => (
        <ol className="mb-2 list-decimal pl-5 last:mb-0">{children}</ol>
      ),
      li: ({ children }: { children?: ReactNode }) => (
        <li className="mb-0.5">{children}</li>
      ),
      strong: ({ children }: { children?: ReactNode }) => (
        <strong className="font-semibold text-(--_dk-text-primary)">
          {children}
        </strong>
      ),
      em: ({ children }: { children?: ReactNode }) => (
        <em className="italic">{children}</em>
      ),
      a: ({ href, children }: { href?: string; children?: ReactNode }) => {
        const nodeKey = citations ? parseKnowledgeRef(href) : null;
        if (nodeKey) {
          return <NodeCitationChip id={nodeKey} label={linkLabel(children) || nodeKey} />;
        }
        if (href && /^file:/i.test(href)) {
          if (!citations) return <>{children}</>;
          return (
            <FileCitationChip href={href} streaming={streaming}>
              {children}
            </FileCitationChip>
          );
        }
        if (citations && href && isHttpCitation(href)) {
          return <WebCitationChip href={href}>{children}</WebCitationChip>;
        }
        return (
          <a
            href={href}
            className="text-(--_dk-accent-hover) underline hover:text-(--_dk-accent-hover)"
            target="_blank"
            rel="noreferrer"
          >
            {children}
          </a>
        );
      },
      table: ({ children }: { children?: ReactNode }) => (
        <div className="agent-markdown-table-wrap mb-2 last:mb-0">
          <table className="agent-markdown-table">{children}</table>
        </div>
      ),
      thead: ({ children }: { children?: ReactNode }) => (
        <thead>{children}</thead>
      ),
      tbody: ({ children }: { children?: ReactNode }) => (
        <tbody>{children}</tbody>
      ),
      tr: ({ children }: { children?: ReactNode }) => <tr>{children}</tr>,
      th: ({ children }: { children?: ReactNode }) => <th>{children}</th>,
      td: ({ children }: { children?: ReactNode }) => <td>{children}</td>,
      code: ({
        className,
        children,
      }: {
        className?: string;
        children?: ReactNode;
      }) => {
        const match = /language-([\w+-]+)/.exec(className ?? "");
        const raw = String(children).replace(/\n$/, "");

        // Fenced code block with a language — delegate to language handler
        if (match) {
          const lang = match[1];
          if (isMermaidLang(lang)) {
            return <MermaidBlock code={raw} streaming={streaming} />;
          }
          return <MarkdownCodeBlock code={raw} lang={lang} />;
        }

        // Fenced code block without language → render as block
        if (raw.includes("\n")) {
          return <MarkdownCodeBlock code={raw} lang="" />;
        }

        // Inline code
        return <code className="agent-markdown-inline-code">{raw}</code>;
      },
      pre: ({ children }: { children?: ReactNode }) => <>{children}</>,
      blockquote: ({ children }: { children?: ReactNode }) => (
        <blockquote className="agent-markdown-blockquote">
          {children}
        </blockquote>
      ),
      // Headings are wired to the project's --_dk-text-* scale (exposed as
      // text-dk-* utilities) so every size lives in one shared system. Body is
      // --_dk-text-base (16px); h3/h2/h1 step up the modular scale. Compact tool
      // cards override these down via .tool-card-markdown in content.css.
      h1: ({ children }: { children?: ReactNode }) => (
        <h1 className="mb-2 text-dk-3xl font-semibold last:mb-0">{children}</h1>
      ),
      h2: ({ children }: { children?: ReactNode }) => (
        <h2 className="mb-2 text-dk-2xl font-semibold last:mb-0">{children}</h2>
      ),
      h3: ({ children }: { children?: ReactNode }) => (
        <h3 className="mb-1 text-dk-xl font-medium last:mb-0">{children}</h3>
      ),
    }),
    [citations, inline, streaming],
  );

  const Tag = inline ? "span" : "div";

  return (
    <Tag className={inline ? "agent-markdown is-inline" : "agent-markdown"}>
      <ReactMarkdown
        remarkPlugins={remarkPlugins}
        urlTransform={transformUrl}
        components={components}
      >
        {displayText}
      </ReactMarkdown>
    </Tag>
  );
});
