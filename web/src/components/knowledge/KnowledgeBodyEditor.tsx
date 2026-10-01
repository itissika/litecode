import type { CSSProperties } from "react";

import { MentionEditor } from "../mention/MentionEditor";

export function KnowledgeBodyEditor({
  label,
  sourceId,
  value,
  candidates,
  rows,
  onChange,
  onBlur,
}: {
  label: string;
  sourceId: string;
  value: string;
  candidates: readonly string[];
  rows: number;
  onChange: (next: string) => void;
  onBlur?: () => void;
}) {
  const style: CSSProperties = { minHeight: `${Math.max(rows, 3) * 1.15}rem` };
  return (
    <MentionEditor
      label={label}
      sourceId={sourceId}
      value={value}
      candidates={candidates}
      className="knowledge-source-input knowledge-source-body"
      style={style}
      symbolLines={false}
      onChange={onChange}
      onBlur={onBlur}
    />
  );
}
