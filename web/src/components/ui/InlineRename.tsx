import { useLayoutEffect, useRef, useState } from "react";

function selectInput(
  input: HTMLInputElement | null,
  selection: "all" | "stem",
): void {
  if (!input) return;
  if (selection === "stem") {
    const dot = input.value.lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : input.value.length);
    return;
  }
  input.select();
}

export interface InlineRenameProps {
  initialValue: string;
  ariaLabel: string;
  maxLength?: number;
  /** What the first focus selects. `stem` keeps a filename extension. */
  selection?: "all" | "stem";
  placeholder?: string;
  /** File-name editors usually want spellcheck off. */
  spellCheck?: boolean;
  /** Commit an empty draft instead of cancelling. */
  allowEmpty?: boolean;
  validate?: (value: string) => string | null;
  onInvalid?: (message: string) => void;
  onCommit: (value: string) => void | Promise<void>;
  onCancel: () => void;
  className?: string;
}

/**
 * Inline single-line editor shared by every renameable row.
 * Enter commits, Escape cancels, blur commits; Esc/blur cannot double-commit.
 */
export function InlineRename({
  initialValue,
  ariaLabel,
  maxLength,
  selection = "all",
  placeholder,
  spellCheck,
  allowEmpty = false,
  validate,
  onInvalid,
  onCommit,
  onCancel,
  className = "",
}: InlineRenameProps) {
  const [draft, setDraft] = useState(initialValue);
  const inputRef = useRef<HTMLInputElement>(null);
  const doneRef = useRef(false);

  useLayoutEffect(() => {
    doneRef.current = false;
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    selectInput(input, selection);
    // Mount-only: the editor is rendered per row and owns one draft.
  }, []);

  const finish = (submit: boolean) => {
    if (doneRef.current) return;
    if (!submit) {
      doneRef.current = true;
      onCancel();
      return;
    }
    const value = draft.trim();
    if (!value && !allowEmpty) {
      doneRef.current = true;
      onCancel();
      return;
    }
    const error = validate?.(value) ?? null;
    if (error) {
      onInvalid?.(error);
      inputRef.current?.focus();
      return;
    }
    doneRef.current = true;
    void onCommit(value);
  };

  return (
    <input
      ref={inputRef}
      value={draft}
      maxLength={maxLength}
      aria-label={ariaLabel}
      placeholder={placeholder}
      spellCheck={spellCheck}
      onClick={(event) => event.stopPropagation()}
      onMouseDown={(event) => event.stopPropagation()}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        // Keep Enter/Escape inside the editor; the row's toggle must not fire.
        event.stopPropagation();
        if (event.key === "Enter") {
          event.preventDefault();
          finish(true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
      className={className}
    />
  );
}
