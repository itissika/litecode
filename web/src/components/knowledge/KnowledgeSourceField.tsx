import { type ReactNode } from "react";

import { KnowledgeBodyEditor } from "./KnowledgeBodyEditor";

/** Card edit glyph. Hidden when the card is idle. */
export type CardSaveMark = "dirty" | "saving" | "saved";

export function KnowledgeSourceField({
  label,
  kind,
  sourceId,
  value,
  candidates,
  onChange,
  rows,
  onBlur,
  singleLine = false,
  className,
}: {
  label: string;
  kind?: "body";
  /** Card whose body this field edits. Mention clicks jump from here. */
  sourceId?: string;
  value: string;
  candidates: string[];
  onChange: (next: string) => void;
  onBlur?: () => void;
  rows: number;
  /** Summary is one fence line: Enter and pasted breaks stay in that line. */
  singleLine?: boolean;
  className?: string;
}) {
  if (kind === "body") {
    return (
      <BodyField
        label={label}
        sourceId={sourceId ?? ""}
        value={value}
        candidates={candidates}
        onChange={onChange}
        onBlur={onBlur}
        rows={rows}
        className={className}
      />
    );
  }
  return (
    <PlainField
      label={label}
      value={value}
      onChange={onChange}
      onBlur={onBlur}
      rows={rows}
      singleLine={singleLine}
      className={className}
    />
  );
}

export function SaveGlyph({ mark }: { mark: CardSaveMark | null }) {
  if (!mark) return null;
  const glyph = mark === "dirty" ? "·" : mark === "saving" ? "…" : "✓";
  const label = mark === "dirty" ? "未保存" : mark === "saving" ? "保存中" : "已保存";
  return (
    <span className="knowledge-save-mark" aria-label={label}>
      {glyph}
    </span>
  );
}

function FieldFrame({
  label,
  className,
  children,
  as: Tag = "div",
}: {
  label: string;
  className?: string;
  children: ReactNode;
  /** Summary uses a label so a click focuses the textarea. The body must not: its remove button would receive that click. */
  as?: "div" | "label";
}) {
  return (
    <Tag
      className={["knowledge-source-field nodrag nowheel", className]
        .filter(Boolean)
        .join(" ")}
    >
      <span className="knowledge-source-label">{label}</span>
      {children}
    </Tag>
  );
}

function PlainField({
  label,
  value,
  onChange,
  onBlur,
  rows,
  singleLine,
  className,
}: {
  label: string;
  value: string;
  onChange: (next: string) => void;
  onBlur?: () => void;
  rows: number;
  singleLine: boolean;
  className?: string;
}) {
  return (
    <FieldFrame label={label} className={className} as="label">
      <textarea
        className="knowledge-source-input"
        rows={rows}
        spellCheck={false}
        value={value}
        onChange={(event) => {
          const next = singleLine
            ? event.target.value.replace(/[\r\n]+/g, " ")
            : event.target.value;
          onChange(next);
        }}
        onKeyDown={(event) => {
          if (singleLine && event.key === "Enter") event.preventDefault();
        }}
        onBlur={() => onBlur?.()}
      />
    </FieldFrame>
  );
}

function BodyField({
  label,
  sourceId,
  value,
  candidates,
  onChange,
  onBlur,
  rows,
  className,
}: {
  label: string;
  sourceId: string;
  value: string;
  candidates: string[];
  onChange: (next: string) => void;
  onBlur?: () => void;
  rows: number;
  className?: string;
}) {
  return (
    <FieldFrame label={label} className={className}>
      <KnowledgeBodyEditor
        label={label}
        sourceId={sourceId}
        value={value}
        candidates={candidates}
        rows={rows}
        onChange={onChange}
        onBlur={onBlur}
      />
    </FieldFrame>
  );
}
