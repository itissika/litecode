import { useEffect, useRef, useState } from "react";
import type {
  CSSProperties,
  PointerEvent as ReactPointerEvent,
  ReactNode,
} from "react";
import { StrategyIcon, TerminalIcon, UsersIcon } from "@phosphor-icons/react";

import { normalizeToolFilePath } from "../api/adapter";
import type { BashJob, SubagentJob } from "../api/types";
import { useBashStore } from "../stores/bashStore";
import { useEditorStore } from "../stores/editorStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSubagentStore } from "../stores/subagentStore";
import { useTurnStore } from "../stores/turnStore";
import { composerCardClass } from "./composerCard";
import { SubagentRosterPanel } from "./SubagentRosterPanel";
import { WaveText } from "./WaveText";

type TodoItemStatus = "pending" | "in_progress" | "completed";
type TodoItem = { id: string; content: string; status: TodoItemStatus };

/** The four session-mount status families the line surfaces. */
export type CapsuleId = "terminal" | "subagent" | "plan" | "todo";

/** Fixed initial height of a vertically-expanded capsule panel (px). */
export const PANEL_INITIAL_H = 160;
export const PANEL_MIN_H = 80;
// Known limitation (F2): this is a static ceiling, not clamped to the dockview
// pane's rect, so on a very short pane a panel dragged to PANEL_MAX_H could have
// its top clipped by the pane's overflow:hidden. Deliberately left as a plain
// static cap — a rect-based clamp needs live layout (unverifiable in jsdom) and
// the reviewer asked not to add a persistent listener for it. Revisit on real-
// device acceptance if a short-pane clip is observed.
export const PANEL_MAX_H = 480;

const EMPTY_BASH_JOBS: BashJob[] = [];
const EMPTY_SUBAGENT_JOBS: SubagentJob[] = [];
const EMPTY_TODO_ITEMS: TodoItem[] = [];

/**
 * One resident row of session-mount status capsules (terminal / subagent /
 * plan / todo). All four capsules are always present — no appear/disappear —
 * and keep the same glass at every state (empty capsules are NOT dimmed).
 *
 * Interaction contract (user-fixed):
 *  1. Resident: four capsules never unmount.
 *  2. Collapsed: one row of icon-only capsules.
 *  3. Hover a capsule → it expands inline to icon + short label + count.
 *  4. Click a capsule → vertically expands that capsule's panel above the row:
 *     fixed initial height, drag handle top-right (drag up to grow, same
 *     pointer-capture pattern as AgentChatInput). Only one panel open at a
 *     time; clicking the same capsule again or clicking outside closes it.
 *  5. The expanded panel hosts the migrated content of the replaced chips
 *     (feature parity): bash reveal, subagent list, plan open, todo list.
 */
export function SessionStatusLine({
  sessionId,
  onRevealBash,
}: {
  sessionId: string;
  onRevealBash?: (callId: string) => void;
}) {
  const bashJobs = useBashStore(
    (s) => s.bySession.get(sessionId)?.jobs ?? EMPTY_BASH_JOBS,
  );
  const subagentJobs = useSubagentStore(
    (s) => s.bySession.get(sessionId)?.jobs ?? EMPTY_SUBAGENT_JOBS,
  );
  const todoItems = useTurnStore(
    (s) => s.byId.get(sessionId)?.todoItems ?? EMPTY_TODO_ITEMS,
  );
  const todoPending = useTurnStore((s) => s.byId.get(sessionId)?.todoPending ?? 0);
  const todoInProgress = useTurnStore(
    (s) => s.byId.get(sessionId)?.todoInProgress ?? 0,
  );
  const todoCompleted = useTurnStore(
    (s) => s.byId.get(sessionId)?.todoCompleted ?? 0,
  );
  const activePlanPath = useTurnStore(
    (s) => s.byId.get(sessionId)?.activePlanPath ?? null,
  );
  const projectRoot = useSessionStore((s) => s.project);
  const openFile = useEditorStore((s) => s.openFile);

  const [openId, setOpenId] = useState<CapsuleId | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const heightRef = useRef(PANEL_INITIAL_H);
  const draggingRef = useRef(false);
  const dragStartRef = useRef({ y: 0, h: PANEL_INITIAL_H });

  // Every freshly-opened panel starts at the fixed initial height. The panel is
  // keyed by `openId`, so switching capsules remounts it; this resets the ref
  // the drag math reads from.
  //
  // The cleanup doubles as the drag-lock release (F3): any close path that
  // unmounts the handle mid-drag — Esc, outside click, re-click, capsule switch,
  // or whole-component unmount — tears this effect down, so `document.body`
  // never stays stuck at ns-resize / user-select:none.
  useEffect(() => {
    heightRef.current = PANEL_INITIAL_H;
    return () => {
      draggingRef.current = false;
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
  }, [openId]);

  // Close on outside mousedown / Escape while a panel is open.
  useEffect(() => {
    if (!openId) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpenId(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpenId(null);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [openId]);

  const toggle = (id: CapsuleId) => setOpenId((cur) => (cur === id ? null : id));

  // Drag handle: dragging up grows the panel (delta = start.y - clientY), same
  // math as AgentChatInput's textarea resize. The new height is written straight
  // to the panel's style (no React state), so dragging never re-renders the
  // panel body.
  const applyResize = (clientY: number) => {
    const delta = dragStartRef.current.y - clientY;
    const next = Math.max(
      PANEL_MIN_H,
      Math.min(PANEL_MAX_H, dragStartRef.current.h + delta),
    );
    heightRef.current = next;
    if (panelRef.current) panelRef.current.style.height = `${next}px`;
  };

  const onResizeStart = (e: ReactPointerEvent) => {
    e.preventDefault();
    draggingRef.current = true;
    dragStartRef.current = { y: e.clientY, h: heightRef.current };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    document.body.style.userSelect = "none";
    document.body.style.cursor = "ns-resize";
  };

  const onResizeMove = (e: ReactPointerEvent) => {
    if (!draggingRef.current) return;
    applyResize(e.clientY);
  };

  const onResizeEnd = (e: ReactPointerEvent) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture?.(e.pointerId)) el.releasePointerCapture?.(e.pointerId);
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
  };

  const openPlan = (path: string) => {
    const resolved = normalizeToolFilePath(path, projectRoot);
    if (resolved) void openFile(resolved);
  };

  const todoTotal = todoPending + todoInProgress + todoCompleted;

  const panelBody =
    openId === "terminal" ? (
      <TerminalPanel jobs={bashJobs} onRevealBash={onRevealBash} />
    ) : openId === "subagent" ? (
      <SubagentRosterPanel sessionId={sessionId} />
    ) : openId === "plan" ? (
      <PlanPanel path={activePlanPath} onOpen={openPlan} />
    ) : openId === "todo" ? (
      <TodoPanelBody
        items={todoItems}
        pending={todoPending}
        inProgress={todoInProgress}
        completed={todoCompleted}
      />
    ) : null;

  return (
    <div
      ref={rootRef}
      className="flex min-w-0 flex-col gap-2"
      data-testid="session-status-line"
    >
      {openId && (
        <div
          key={openId}
          ref={panelRef}
          data-testid="status-capsule-panel"
          data-capsule={openId}
          style={{ height: PANEL_INITIAL_H }}
          className={`${composerCardClass} relative overflow-hidden`}
        >
          <button
            type="button"
            aria-label="Resize panel"
            data-testid="status-panel-resize"
            onPointerDown={onResizeStart}
            onPointerMove={onResizeMove}
            onPointerUp={onResizeEnd}
            onPointerCancel={onResizeEnd}
            className="absolute right-1.5 top-1.5 z-10 flex h-4 w-6 cursor-ns-resize items-center justify-center rounded text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary)"
          >
            <span aria-hidden className="block h-0.5 w-3 rounded-full bg-current" />
          </button>
          <div className="h-full overflow-y-auto py-2">{panelBody}</div>
        </div>
      )}

      <div
        className="flex min-w-0 items-center gap-2"
        data-testid="session-status-capsules"
      >
        <Capsule
          id="terminal"
          open={openId === "terminal"}
          onToggle={toggle}
          icon={
            <TerminalIcon
              size={14}
              weight="fill"
              aria-hidden
              className={bashJobs.length > 0 ? "terminal-status-icon" : ""}
            />
          }
          label="Terminals"
          count={bashJobs.length}
          ariaLabel={`Terminal status, ${bashJobs.length} active`}
        />
        <Capsule
          id="subagent"
          open={openId === "subagent"}
          onToggle={toggle}
          icon={
            <UsersIcon
              size={14}
              weight="fill"
              aria-hidden
              className={subagentJobs.length > 0 ? "subagent-status-icon" : ""}
            />
          }
          label="Workers"
          count={subagentJobs.length}
          ariaLabel={`Subagent status, ${subagentJobs.length} running`}
        />
        <Capsule
          id="plan"
          open={openId === "plan"}
          onToggle={toggle}
          icon={<StrategyIcon size={14} weight="fill" aria-hidden />}
          label="Plan"
          ariaLabel="Session plan"
        />
        <Capsule
          id="todo"
          open={openId === "todo"}
          onToggle={toggle}
          icon={
            <ProgressRing
              pct={
                todoTotal > 0
                  ? Math.round((todoCompleted / todoTotal) * 100)
                  : 0
              }
            />
          }
          label="Tasks"
          count={todoTotal}
          ariaLabel={`Task status, ${todoTotal} ${
            todoTotal === 1 ? "task" : "tasks"
          }`}
        />
      </div>
    </div>
  );
}

/** A single resident capsule. Collapsed it shows only its glyph; hovering it
 *  (or having its own vertical panel open) expands it inline to glyph + label +
 *  count. The expansion is in-flow, so the resident row simply grows rightward
 *  into the free trailing space rather than overlaying anything. */
function Capsule({
  id,
  open,
  onToggle,
  icon,
  label,
  count,
  ariaLabel,
}: {
  id: CapsuleId;
  open: boolean;
  onToggle: (id: CapsuleId) => void;
  icon: ReactNode;
  label: string;
  count?: number;
  ariaLabel: string;
}) {
  const [hovered, setHovered] = useState(false);
  // Expanded while hovered, and pinned expanded while this capsule's own panel
  // is open, so the labelled capsule stays attached to the panel it owns.
  const expanded = hovered || open;

  return (
    <button
      type="button"
      data-testid={`capsule-${id}`}
      data-open={open}
      data-expanded={expanded}
      aria-expanded={open}
      aria-label={ariaLabel}
      onClick={() => onToggle(id)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className={`${composerCardClass} flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 px-2.5 text-xs text-(--_dk-text-secondary) transition-colors duration-100 active:brightness-90 ${
        open
          ? "border-(--_dk-line-visible) text-(--_dk-text-primary)"
          : "hover:text-(--_dk-text-primary)"
      }`}
    >
      <span className="flex shrink-0 items-center">{icon}</span>
      {expanded && (
        <>
          <span className="shrink-0 whitespace-nowrap">{label}</span>
          {count != null && (
            <span className="shrink-0 font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)">
              ×{count}
            </span>
          )}
        </>
      )}
    </button>
  );
}

function PanelEmpty({ children }: { children: ReactNode }) {
  return (
    <div className="px-3 py-2 text-xs italic text-(--_dk-text-disabled)">
      {children}
    </div>
  );
}

/** Migrated terminal chip content: the alive bash jobs, each clickable to
 *  reveal its live view (preserves the old chip's click-to-cycle affordance). */
function TerminalPanel({
  jobs,
  onRevealBash,
}: {
  jobs: BashJob[];
  onRevealBash?: (callId: string) => void;
}) {
  if (jobs.length === 0) return <PanelEmpty>No active terminals</PanelEmpty>;
  return (
    <ul className="space-y-0.5 px-1.5">
      {jobs.map((job) => (
        <li key={job.id}>
          <button
            type="button"
            aria-label={`Reveal terminal: ${job.command_preview}`}
            onClick={() => onRevealBash?.(job.call_id)}
            className="flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-xs text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-primary)"
          >
            <TerminalIcon
              size={13}
              weight="fill"
              aria-hidden
              className="terminal-status-icon shrink-0"
            />
            <span className="min-w-0 flex-1 truncate font-mono">
              {job.command_preview}
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/** Migrated plan chip content: the active plan path + an Open affordance that
 *  resolves the workspace path exactly like the old chip did. */
function PlanPanel({
  path,
  onOpen,
}: {
  path: string | null;
  onOpen: (path: string) => void;
}) {
  if (!path) return <PanelEmpty>No active plan</PanelEmpty>;
  return (
    <div className="flex flex-col gap-2 px-3 py-1">
      <div className="truncate font-mono text-xs text-(--_dk-text-secondary)">
        {path}
      </div>
      <button
        type="button"
        onClick={() => onOpen(path)}
        className="flex w-fit items-center gap-1.5 rounded border border-(--_dk-line) px-2 py-1 text-xs text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-primary)"
      >
        <StrategyIcon size={13} weight="fill" aria-hidden />
        Open plan
      </button>
    </div>
  );
}

/** Migrated TodoPanel content (ring + current task + full list). */
function TodoPanelBody({
  items,
  pending,
  inProgress,
  completed,
}: {
  items: TodoItem[];
  pending: number;
  inProgress: number;
  completed: number;
}) {
  const total = pending + inProgress + completed;
  const pct = total > 0 ? Math.round((completed / total) * 100) : 0;
  const current = items.find((item) => item.status === "in_progress");
  return (
    <div className="px-3 py-1 text-xs">
      <div className="flex items-center gap-2 py-1 text-(--_dk-text-muted)">
        <ProgressRing pct={pct} />
        <span className="min-w-0 flex-1 truncate">
          {current ? (
            <WaveText
              text={current.content}
              className="todo-wave-text"
              charClass="todo-wave-char"
            />
          ) : (
            <span className="italic text-(--_dk-text-disabled)">
              No active task
            </span>
          )}
        </span>
        <span className="shrink-0 font-mono text-dk-xs tabular-nums">
          {completed}/{total}
        </span>
      </div>
      {items.length === 0 ? (
        <div className="py-1 italic text-(--_dk-text-disabled)">
          No tasks yet
        </div>
      ) : (
        <div className="space-y-1 py-1">
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

