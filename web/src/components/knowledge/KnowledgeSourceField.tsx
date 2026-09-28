import { useEffect, useMemo, useRef, useState } from "react";

import {
  applyCompletion,
  completionAt,
  type KnowledgeCompletion,
} from "../../lib/knowledge/complete";

export function KnowledgeSourceField({
  label,
  kind,
  value,
  candidates,
  onChange,
  rows,
  onBlur,
  singleLine = false,
  className,
}: {
  label: string;
  kind?: "refs" | "body";
  value: string;
  candidates: string[];
  onChange: (next: string) => void;
  onBlur?: () => void;
  rows: number;
  /** Summary is one fence line: Enter and pasted breaks stay in that line. */
  singleLine?: boolean;
  className?: string;
}) {
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [caret, setCaret] = useState(0);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const completion = useMemo(
    () => (open && kind ? completionAt(kind, value, caret, candidates) : null),
    [open, kind, value, caret, candidates],
  );
  const items = completion?.items ?? [];

  useEffect(() => {
    setActive(0);
  }, [items.join("\0")]);

  function choose(completion: KnowledgeCompletion, key: string) {
    if (!kind) return;
    const applied = applyCompletion(kind, value, completion, key);
    onChange(applied.text);
    setCaret(applied.caret);
    setOpen(false);
    requestAnimationFrame(() => {
      const input = inputRef.current;
      if (!input) return;
      input.focus();
      input.setSelectionRange(applied.caret, applied.caret);
    });
  }

  return (
    <label
      className={["knowledge-source-field nodrag nowheel", className]
        .filter(Boolean)
        .join(" ")}
    >
      <span className="knowledge-source-label">{label}</span>
      <textarea
        ref={inputRef}
        className="knowledge-source-input"
        rows={rows}
        spellCheck={false}
        value={value}
        onChange={(event) => {
          const next = singleLine
            ? event.target.value.replace(/[\r\n]+/g, " ")
            : event.target.value;
          onChange(next);
          setCaret(event.target.selectionStart ?? next.length);
          setOpen(Boolean(kind));
        }}
        onClick={(event) => {
          setCaret(event.currentTarget.selectionStart ?? 0);
          setOpen(Boolean(kind));
        }}
        onKeyUp={(event) => {
          if (event.key === "ArrowUp" || event.key === "ArrowDown") return;
          setCaret(event.currentTarget.selectionStart ?? 0);
        }}
        onBlur={() => {
          onBlur?.();
          window.setTimeout(() => setOpen(false), 120);
        }}
        onKeyDown={(event) => {
          if (singleLine && event.key === "Enter") {
            event.preventDefault();
            return;
          }
          if (!completion || items.length === 0) return;
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setActive((index) => (index + 1) % items.length);
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            setActive((index) => (index - 1 + items.length) % items.length);
          } else if (event.key === "Enter" || event.key === "Tab") {
            event.preventDefault();
            const key = items[active];
            if (key) choose(completion, key);
          } else if (event.key === "Escape") {
            event.preventDefault();
            setOpen(false);
          }
        }}
      />
      {completion && items.length > 0 ? (
        <ul className="knowledge-complete-list" role="listbox">
          {items.slice(0, 8).map((key, index) => (
            <li key={key}>
              <button
                type="button"
                role="option"
                aria-selected={index === active}
                className={
                  index === active
                    ? "knowledge-complete-item is-active"
                    : "knowledge-complete-item"
                }
                onMouseDown={(event) => {
                  event.preventDefault();
                  choose(completion, key);
                }}
              >
                {key}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </label>
  );
}
