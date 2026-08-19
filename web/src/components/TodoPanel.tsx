import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";

import { useTurnStore } from "../stores/turnStore";
import { composerCardClass } from "./composerCard";
import { WaveText } from "./WaveText";

type TodoItemStatus = "pending" | "in_progress" | "completed";

export function TodoPanel({ sessionId }: { sessionId: string }) {
  const pending = useTurnStore(
    (s) => s.byId.get(sessionId)?.todoPending ?? 0,
  );
  const inProgress = useTurnStore(
    (s) => s.byId.get(sessionId)?.todoInProgress ?? 0,
  );
  const completed = useTurnStore(
    (s) => s.byId.get(sessionId)?.todoCompleted ?? 0,
  );
  const items = useTurnStore(
    (s) => s.byId.get(sessionId)?.todoItems ?? EMPTY_TODO_ITEMS,
  );
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  const total = pending + inProgress + completed;
  const pct = total > 0 ? Math.round((completed / total) * 100) : 0;
  const current = items.find((item) => item.status === "in_progress");

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div
      ref={rootRef}
      className={`${composerCardClass} overflow-hidden`}
    >
      {open ? (
        <div className="max-h-48 overflow-y-auto px-3 py-2 text-xs">
          {items.length === 0 ? (
            <div className="py-1 italic text-(--_dk-text-disabled)">No tasks yet</div>
          ) : (
            <div className="space-y-1">
              {items.map((item) => (
                <div key={item.id} className="flex items-start gap-2">
                  <TodoStatusIcon status={item.status} />
                  <span
                    className={
                      item.status === "completed"
                        ? "text-(--_dk-text-disabled) line-through"
                        : "text-(--_dk-text-secondary)"
                    }
                  >
                    {item.content}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      ) : null}
      {open ? <div className="mx-3 border-t border-(--_dk-line)" /> : null}
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex h-[30px] w-full items-center gap-2 px-3 text-left text-xs text-(--_dk-text-muted) transition-transform duration-100 hover:brightness-110 active:scale-[0.98] active:brightness-90"
      >
        <ProgressRing pct={pct} />
        <span className="min-w-0 flex-1 truncate">
          {current ? (
            <WaveText
              text={current.content}
              className="todo-wave-text"
              charClass="todo-wave-char"
            />
          ) : (
            <span className="italic text-(--_dk-text-disabled)">No active task</span>
          )}
        </span>
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="shrink-0 text-(--_dk-text-disabled) transition-transform duration-150"
          style={{ transform: open ? "rotate(90deg)" : "rotate(-90deg)" }}
        >
          <path d="M3 1.5l4 3.5-4 3.5" />
        </svg>
      </button>
    </div>
  );
}

function ProgressRing({ pct }: { pct: number }) {
  const size = 14;
  const stroke = 2;
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const arc = (pct / 100) * c;
  const offset = c - arc;
  const head = Math.min(c * 0.18, arc);
  const cx = size / 2;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className="shrink-0 overflow-visible"
      aria-hidden
    >
      <circle
        cx={cx}
        cy={cx}
        r={r}
        stroke="var(--_dk-line)"
        strokeWidth={stroke}
        fill="none"
      />
      <circle
        cx={cx}
        cy={cx}
        r={r}
        stroke="var(--_dk-emerald-500)"
        strokeWidth={stroke}
        fill="none"
        strokeLinecap="round"
        strokeDasharray={c}
        strokeDashoffset={offset}
        transform={`rotate(-90 ${cx} ${cx})`}
        style={{ transition: "stroke-dashoffset 300ms" }}
      />
      {head > 0 ? (
        <circle
          className="todo-pulse"
          cx={cx}
          cy={cx}
          r={r}
          strokeWidth={stroke}
          fill="none"
          strokeLinecap="round"
          strokeDasharray={`${head} ${c}`}
          transform={`rotate(-90 ${cx} ${cx})`}
          style={{ "--todo-pulse-travel": `${arc - head}px` } as CSSProperties}
        />
      ) : null}
    </svg>
  );
}

function TodoStatusIcon({ status }: { status: TodoItemStatus }) {
  if (status === "completed") {
    return (
      <span className="mt-0.5 h-3 w-3 shrink-0 rounded-full bg-(--_dk-emerald-500)" />
    );
  }
  if (status === "in_progress") {
    return (
      <span className="mt-0.5 h-3 w-3 shrink-0 rounded-full border-2 border-(--_dk-emerald-500) bg-(--_dk-emerald-500)/30" />
    );
  }
  return (
    <span className="mt-0.5 h-3 w-3 shrink-0 rounded-full border border-(--_dk-line-visible)" />
  );
}

const EMPTY_TODO_ITEMS: {
  id: string;
  content: string;
  status: TodoItemStatus;
}[] = [];
