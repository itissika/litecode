import { createContext, useContext } from "react";
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
} from "react-markdown";
import remarkGfm from "remark-gfm";

import { chipForMarker } from "../../lib/knowledge/refDisplay";
import { knowledgeFirstLineSegments } from "../../lib/knowledge/markers";
import {
  parseKnowledgeRef,
  remarkKnowledgeRef,
} from "../../lib/knowledge/remarkKnowledgeRef";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeRefChip } from "./KnowledgeRefChip";

const KnowledgeSourceContext = createContext<number | null>(null);

function knowledgeUrlTransform(url: string): string {
  if (parseKnowledgeRef(url)) return url;
  return defaultUrlTransform(url);
}

function BodyRefChip({ marker }: { marker: string }) {
  const sourceId = useContext(KnowledgeSourceContext);
  const source = useKnowledgeStore((s) =>
    sourceId == null ? undefined : s.byId.get(sourceId),
  );
  const byKey = useKnowledgeStore((s) => s.byKey);
  const focusCanvas = useKnowledgeStore((s) => s.focusCanvas);
  if (!source) return <span>{marker}</span>;
  const model = chipForMarker(source, marker, byKey);
  return (
    <KnowledgeRefChip
      model={model}
      onActivate={() => {
        if (model.targetId != null) focusCanvas(model.targetId);
      }}
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
    if (key) return <BodyRefChip marker={key} />;
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

/** One prose line with `[[key]]` chips — for graph card summaries. */
export function KnowledgeInlineBody({
  sourceId,
  text,
}: {
  sourceId: number;
  text: string;
}) {
  const segments = knowledgeFirstLineSegments(text);
  return (
    <KnowledgeSourceContext.Provider value={sourceId}>
      <div className="knowledge-inline-body">
        {segments.map((segment, index) => {
          if (segment.type === "text") {
            return segment.value ? (
              <span key={index} className="knowledge-inline-text">
                {segment.value}
              </span>
            ) : null;
          }
          return <BodyRefChip key={index} marker={segment.key} />;
        })}
      </div>
    </KnowledgeSourceContext.Provider>
  );
}

export function KnowledgeMarkdown({
  sourceId,
  text,
}: {
  sourceId: number;
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

export function knowledgeStatusLabel(
  status: "enabled" | "disabled" | "pending",
): string {
  if (status === "disabled") return "禁用";
  if (status === "pending") return "待审阅";
  return "启用";
}

export function KnowledgeStatusBadge({
  status,
}: {
  status: "enabled" | "disabled" | "pending";
}) {
  return (
    <span className={`knowledge-status is-${status}`}>
      {knowledgeStatusLabel(status)}
    </span>
  );
}
