import { memo, useMemo, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { citationUrlTransform, classifyWorkspaceLink } from "../lib/citationRef";
import { fileKindFromPath } from "../lib/fileKind";
import { isWorkspaceFileRef } from "../lib/knowledge/markers";
import { followCitation } from "../lib/knowledge/panel";
import { resolveMarkdownImage } from "../lib/markdownImagePath";
import {
  parseKnowledgeRef,
  parseLocationRef,
  remarkKnowledgeRef,
} from "../lib/knowledge/remarkKnowledgeRef";
import type { Citation } from "../lib/knowledge/refDisplay";
import { isMermaidLang } from "../lib/mermaid";
import { CitationChip } from "./CitationChip";
import { CodeBlock } from "./CodeBlock";
import { MermaidBlock } from "./MermaidBlock";
import { WorkspacePicture } from "./WorkspacePicture";

function linkLabel(children: ReactNode): string {
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map((child) => linkLabel(child)).join("");
  return "";
}

/** File preview. Absent callers keep today's chat rendering. */
export interface MarkdownPreview {
  /** Workspace path of the markdown file. Image paths resolve against its directory. */
  basePath: string;
}

/**
 * Whole-file image citation. Symbol and line references stay text capsules.
 * The path is already workspace-root relative; it is not joined to the markdown file.
 */
function wholeFileImagePath(located: {
  path: string;
  symbol: string | null;
  line: number | null;
  lines?: string | null;
}): string | null {
  if (located.symbol?.trim()) return null;
  if (located.line != null) return null;
  if (located.lines?.trim()) return null;
  if (!isWorkspaceFileRef(located.path)) return null;
  if (fileKindFromPath(located.path) !== "image") return null;
  return located.path;
}

function FileCitation({
  citation,
  imagePath,
  sourceId,
}: {
  citation: Citation;
  imagePath: string | null;
  sourceId?: string;
}) {
  return (
    <>
      <CitationChip citation={citation} sourceId={sourceId} />
      {imagePath ? (
        <WorkspacePicture path={imagePath} onOpen={() => followCitation(citation)} />
      ) : null}
    </>
  );
}

export interface MarkdownBodyProps {
  text: string;
  streaming?: boolean;
  /** Assistant prose only. Other callers keep ordinary links. */
  citations?: boolean;
  /** Sit in the surrounding line. Used when a mention capsule shares the sentence. */
  inline?: boolean;
  /**
   * Markdown file preview. Turns on shortcode chips, workspace images, and
   * pictures for whole-file image chips. Ordinary markdown links stay links.
   */
  preview?: MarkdownPreview;
  /**
   * Knowledge node whose snapshot holds this body's citation verdicts. Set only
   * where the text belongs to that node; without it a chip shows its reference
   * and no state (KR-INV-3: verdicts come from the snapshot, never locally).
   */
  sourceId?: string;
}

export const MarkdownBody = memo(function MarkdownBody({
  text,
  streaming = false,
  citations = false,
  inline = false,
  sourceId,
  preview,
}: MarkdownBodyProps) {
  const basePath = preview?.basePath;
  const shortcodes = citations || basePath != null;
  const remarkPlugins = useMemo(
    () => (shortcodes ? [remarkGfm, remarkKnowledgeRef] : [remarkGfm]),
    [shortcodes],
  );
  const transformUrl = useMemo(
    () => (value: string) => {
      if (shortcodes && (parseKnowledgeRef(value) || parseLocationRef(value))) return value;
      return citationUrlTransform(value);
    },
    [shortcodes],
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
        const nodeKey = shortcodes ? parseKnowledgeRef(href) : null;
        if (nodeKey) {
          return (
            <CitationChip
              citation={{ kind: "node", key: nodeKey }}
              sourceId={sourceId}
              label={linkLabel(children) || nodeKey}
            />
          );
        }
        const located = shortcodes ? parseLocationRef(href) : null;
        if (located) {
          const citation: Citation = {
            kind: "file",
            path: located.path,
            symbol: located.symbol,
            line: located.line,
            lines: located.lines,
          };
          return (
            <FileCitation
              citation={citation}
              sourceId={sourceId}
              imagePath={basePath != null ? wholeFileImagePath(located) : null}
            />
          );
        }
        const workspaceLink = citations ? classifyWorkspaceLink(href) : null;
        if (workspaceLink?.action === "chip") {
          return (
            <CitationChip
              citation={{
                kind: "file",
                path: workspaceLink.path,
                symbol: workspaceLink.symbol,
                line: workspaceLink.line,
              }}
              sourceId={sourceId}
            />
          );
        }
        if (workspaceLink?.action === "text") return <>{children}</>;
        if (href && /^file:/i.test(href)) {
          return <>{children}</>;
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
          return <CodeBlock code={raw} lang={lang} />;
        }

        // Fenced code block without language → render as block
        if (raw.includes("\n")) {
          return <CodeBlock code={raw} lang="" />;
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
      ...(basePath != null
        ? {
            img: ({ src, alt }: { src?: string; alt?: string }) => {
              const target = resolveMarkdownImage(basePath, src);
              if (target?.kind === "remote") {
                return <img src={target.url} alt={alt ?? ""} />;
              }
              if (target?.kind === "workspace") {
                return <WorkspacePicture path={target.path} alt={alt} />;
              }
              return alt ? <span>{alt}</span> : null;
            },
          }
        : {}),
    }),
    [basePath, citations, inline, shortcodes, sourceId, streaming],
  );

  const Tag = inline ? "span" : "div";

  return (
    <Tag className={inline ? "agent-markdown is-inline" : "agent-markdown"}>
      <ReactMarkdown
        remarkPlugins={remarkPlugins}
        urlTransform={transformUrl}
        components={components}
      >
        {text}
      </ReactMarkdown>
    </Tag>
  );
});
