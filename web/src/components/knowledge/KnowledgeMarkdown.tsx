import { createContext, useContext } from "react";
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
} from "react-markdown";
import remarkGfm from "remark-gfm";

import { normalizeKey } from "../../lib/knowledge/markers";
import {
  parseKnowledgeRef,
  remarkKnowledgeRef,
} from "../../lib/knowledge/remarkKnowledgeRef";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

const KnowledgeSourceContext = createContext<number | null>(null);

function knowledgeUrlTransform(url: string): string {
  if (parseKnowledgeRef(url)) return url;
  return defaultUrlTransform(url);
}

function KnowledgeRefChip({ marker }: { marker: string }) {
  const sourceId = useContext(KnowledgeSourceContext);
  const target = useKnowledgeStore((s) => s.byKey.get(normalizeKey(marker)));
  const source = useKnowledgeStore((s) =>
    sourceId == null ? undefined : s.byId.get(sourceId),
  );
  const focus = useKnowledgeStore((s) => s.focus);

  let invalid: string | null = null;
  let warning: string | null = null;
  if (!target) invalid = `键「${marker}」不存在`;
  else if (!source) invalid = "没有来源节点";
  else if (target.id === source.id) invalid = "不能引用自己";
  else if (!source.relations.includes(target.id)) {
    invalid = `「${marker}」未在关系列登记`;
  } else if (source.status === "enabled" && target.status !== "enabled") {
    warning =
      target.status === "disabled"
        ? `「${marker}」已禁用`
        : `「${marker}」待审阅`;
  }

  return (
    <button
      type="button"
      className={
        invalid
          ? "knowledge-ref is-invalid"
          : warning
            ? "knowledge-ref is-warning"
            : "knowledge-ref"
      }
      title={invalid ?? warning ?? `打开 ${marker}`}
      aria-invalid={invalid ? true : undefined}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (!invalid && target) focus(target.id);
      }}
    >
      {marker}
    </button>
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
    if (key) return <KnowledgeRefChip marker={key} />;
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
