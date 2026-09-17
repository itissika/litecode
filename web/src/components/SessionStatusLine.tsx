import {
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  CSSProperties,
  PointerEvent as ReactPointerEvent,
  ReactNode,
} from "react";
import { StrategyIcon, TerminalIcon, UsersIcon } from "@phosphor-icons/react";

import { normalizeToolFilePath } from "../api/adapter";
import type { BashJob } from "../api/types";
import { readFile } from "../api/workspace";
import { useBashStore } from "../stores/bashStore";
import { useEditorStore } from "../stores/editorStore";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTurnStore } from "../stores/turnStore";
import { composerCardClass } from "./composerCard";
import { AgentMarkdown } from "./AgentMarkdown";
import { SubagentRosterPanel } from "./SubagentRosterPanel";
import { WaveText } from "./WaveText";

type TodoItemStatus = "pending" | "in_progress" | "completed";
type TodoItem = { id: string; content: string; status: TodoItemStatus };

/** Icon-only capsule width; the expanded capsule receives remaining row space. */
const CAPSULE_BASE_PX = 36;

/** The four session-mount status families the line surfaces. */
export type CapsuleId = "terminal" | "subagent" | "plan" | "todo";

/** Fixed initial height of a vertically-expanded capsule panel (px). */
export const PANEL_INITIAL_H = 160;
export const PANEL_MIN_H = 80;
/** Exit-animation duration (ms) — matches `status-panel-exit` in chat.css. */
export const PANEL_EXIT_MS = 160;
// Known limitation (F2): this is a static ceiling, not clamped to the dockview
// pane's rect, so on a very short pane a panel dragged to PANEL_MAX_H could have
// its top clipped by the pane's overflow:hidden. Deliberately left as a plain
// static cap — a rect-based clamp needs live layout (unverifiable in jsdom) and
// the reviewer asked not to add a persistent listener for it. Revisit on real-
// device acceptance if a short-pane clip is observed.
export const PANEL_MAX_H = 480;

const EMPTY_BASH_JOBS: BashJob[] = [];
const EMPTY_TODO_ITEMS: TodoItem[] = [];

/**
 * One resident row of session-mount status capsules (terminal / subagent /
 * plan / todo). All four capsules are always present — no appear/disappear —
 * and keep the same glass at every state (empty capsules are NOT dimmed).
 *
 * Two-level expansion (user-fixed):
 *  1. Resident: four capsules never unmount; widths are adaptive with only
 *     per-state min-widths (collapsed = icon floor, expanded = label floor).
 *  2. Level 1 — horizontal: exactly one capsule is inline-expanded at any
 *     time. The expanded capsule stretches to fill the row (width animated
 *     from its collapsed icon-only size, content cross-fading) and shows
 *     icon + label + rich detail + count (todo: current task + progress,
 *     plan: active path, subagent: running count, bash: latest command). The
 *     slot is public mutable state: hovering a capsule claims it (sticky — it
 *     stays after the mouse leaves), and while idle (no hover, no panel open)
 *     a data change in any capsule's domain claims the slot for that capsule
 *     (attention cue). Todo owns the slot by default.
 *  3. Level 2 — vertical: clicking a capsule expands its panel above the row:
 *     fixed initial height, drag handle top-right (drag up to grow, same
 *     pointer-capture pattern as AgentChatInput). Only one panel open at a
 *     time; clicking the same capsule again or clicking outside closes it.
 *     While a panel is already open, hover follows onto that capsule's panel.
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
  // Minimal worker summary: total children and the live running count.
  // Children are counted from the session list (durable — the roster panel
  // and the capsule share the same lifecycle: children appear there exactly
  // as their parent does, surviving a reload), with bindings/jobs covering
  // the transient window right after `agent/subagent_bound`.
  const subagentBindings = useMessageStore(
    (s) => s.bySession.get(sessionId)?.subagentBindings,
  );
  const sessions = useSessionStore((s) => s.sessions);
  const childSessions = useMemo(
    () => sessions.filter((sess) => sess.parent_session_id === sessionId),
    [sessions, sessionId],
  );
  const subagentTotal = Math.max(
    childSessions.length,
    subagentBindings
      ? new Set(Object.values(subagentBindings).filter(Boolean)).size
      : 0,
  );
  const subagentRunning = Math.max(
    childSessions.filter(
      (s) => s.running === true || s.status === "running" || s.status === "stopping",
    ).length,
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
  // Level-1 horizontal expansion: exactly one capsule owns the slot at any
  // time. Public mutable — claimed by hover (sticky) or, while idle, by a data
  // change in that capsule's domain. Todo owns the slot by default.
  const [expandedId, setExpandedId] = useState<CapsuleId>("todo");
  // Closing keeps the panel mounted so its shrink-back animation can play:
  // `openId` is nulled immediately and the mount is torn down on animationend.
  const [closingId, setClosingId] = useState<CapsuleId | null>(null);
  // Teardown safety net for the closing mount (below): onAnimationEnd is the
  // primary path, but if the browser suppresses the animation (reduced-motion,
  // backgrounded tab) the timer guarantees the mount never lingers.
  const closeTimerRef = useRef<number | null>(null);
  const finishClose = () => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    setClosingId(null);
  };
  // Which capsule the pointer currently rests on (ref: the data-change watcher
  // reads it without needing hover to be render state).
  const hoverRef = useRef<CapsuleId | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const heightRef = useRef(PANEL_INITIAL_H);
  const draggingRef = useRef(false);
  const dragStartRef = useRef({ y: 0, h: PANEL_INITIAL_H });

  // Flexbox owns width calculation: every capsule has the same icon-only
  // basis, while the expanded one receives all remaining row space. Animating
  // flex-grow preserves the focus hand-off without measuring the row in JS.
  const rowRef = useRef<HTMLDivElement>(null);

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
      if (!rootRef.current?.contains(e.target as Node)) requestClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") requestClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [openId]);

  // A close keeps the panel mounted for its exit animation. `openId` drops to
  // null immediately (outside/Escape handlers are keyed on it), while the
  // mount lives on as `closingId` until `animationend`/timer tears it down.
  const requestClose = () => {
    if (openId && !closingId) {
      setClosingId(openId);
      closeTimerRef.current = window.setTimeout(finishClose, PANEL_EXIT_MS);
    }
    setOpenId(null);
  };

  const toggle = (id: CapsuleId) => {
    // Clicking also claims the horizontal slot so the labelled capsule stays
    // attached to the panel it owns.
    setExpandedId(id);
    if (openId === id) {
      requestClose();
    } else {
      // A switch tears down any closing mount and mounts the new panel
      // directly (no exit/enter overlap — the subagent panel must not double-
      // mount its subscription + virtualized list).
      finishClose();
      setOpenId(id);
    }
  };

  const onHoverStart = (id: CapsuleId) => {
    hoverRef.current = id;
    setExpandedId(id);
    // While a panel is already open, hover follows: the panel tracks the
    // hovered capsule. A closed panel still waits for a click (level-1 vs
    // level-2 remain distinct gestures).
    setOpenId((cur) => (cur ? id : cur));
  };
  const onHoverEnd = (id: CapsuleId) => {
    if (hoverRef.current === id) hoverRef.current = null;
  };

  // Level-1 attention trigger: while idle (no hover, no panel open), a value
  // change in any capsule's domain claims the horizontal slot for that
  // capsule. Signatures are compared by value, not array identity — snapshot
  // replays must not fire the trigger. Changes observed while busy are
  // consumed, not queued.
  const sigRef = useRef<{ bash: string; sub: string; plan: string; todo: string } | null>(null);
  useEffect(() => {
    const cur = {
      bash: bashJobs.map((j) => j.id).join(","),
      sub: `${childSessions.map((session) => `${session.id}:${session.status}:${session.running}`).join(",")}|${subagentTotal}`,
      plan: activePlanPath ?? "",
      todo: todoItems.map((i) => `${i.id}:${i.status}:${i.content}`).join("|"),
    };
    const prev = sigRef.current;
    sigRef.current = cur;
    if (!prev || hoverRef.current !== null || openId !== null) return;
    if (prev.todo !== cur.todo) setExpandedId("todo");
    else if (prev.plan !== cur.plan) setExpandedId("plan");
    else if (prev.sub !== cur.sub) setExpandedId("subagent");
    else if (prev.bash !== cur.bash) setExpandedId("terminal");
  }, [bashJobs, childSessions, subagentTotal, activePlanPath, todoItems, openId]);

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

  const todoCurrent = todoItems.find((item) => item.status === "in_progress");
  const todoTotal = todoPending + todoInProgress + todoCompleted;

  const panelBody =
    (openId ?? closingId) === "terminal" ? (
      <TerminalPanel jobs={bashJobs} onRevealBash={onRevealBash} />
    ) : (openId ?? closingId) === "subagent" ? (
      <SubagentRosterPanel sessionId={sessionId} />
    ) : (openId ?? closingId) === "plan" ? (
      <PlanPanel
        path={activePlanPath}
        projectRoot={projectRoot}
        onOpen={openPlan}
      />
    ) : (openId ?? closingId) === "todo" ? (
      <TodoPanelBody
        items={todoItems}
        pending={todoPending}
        inProgress={todoInProgress}
        completed={todoCompleted}
      />
    ) : null;

  // The panel morphs from the owning capsule: transform-origin x is the
  // capsule's collapsed-pill center within the row (the capsule's left edge is
  // stable whether it is expanded or not), y is the panel's bottom edge, which
  // sits right above the row — so growth reads as coming out of the button.
  // jsdom reports zero rects; the origin then collapses to the row's left.
  const panelId = openId ?? closingId;
  let originX = CAPSULE_BASE_PX / 2;
  const rowEl = rowRef.current;
  if (panelId && rowEl) {
    const capsuleEl = rowEl.querySelector<HTMLElement>(
      `[data-testid="capsule-${panelId}"]`,
    );
    if (capsuleEl) {
      const er = capsuleEl.getBoundingClientRect();
      const rr = rowEl.getBoundingClientRect();
      originX = er.left - rr.left + CAPSULE_BASE_PX / 2;
    }
  }

  return (
    <div
      ref={rootRef}
      className="flex min-w-0 flex-col gap-2"
      data-testid="session-status-line"
    >
      {panelId && (
        <div
          key={panelId}
          ref={panelRef}
          data-testid="status-capsule-panel"
          data-capsule={panelId}
          style={{
            // A fresh open starts at the fixed initial height; the closing
            // mount keeps the dragged height it was shut at.
            height: openId ? PANEL_INITIAL_H : heightRef.current,
            transformOrigin: `${originX}px 100%`,
          }}
          className={`${composerCardClass} relative overflow-hidden [container-type:size] ${
            openId ? "status-panel-enter" : "status-panel-exit"
          }`}
          onAnimationEnd={
            !openId ? finishClose : undefined
          }
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
          <div
            className="h-full overflow-y-auto overscroll-contain py-2"
            data-testid="status-panel-scroll"
          >
            {panelBody}
          </div>
        </div>
      )}

      <div
        ref={rowRef}
        className="flex min-w-0 items-center gap-2"
        data-testid="session-status-capsules"
      >
        <Capsule
          id="todo"
          open={openId === "todo"}
          expanded={expandedId === "todo"}
          onToggle={toggle}
          onHoverStart={onHoverStart}
          onHoverEnd={onHoverEnd}
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
          detail={
            todoCurrent ? (
              <WaveText
                text={todoCurrent.content}
                className="todo-wave-text"
                charClass="todo-wave-char"
              />
            ) : (
              <span className="italic text-(--_dk-text-disabled)">
                No active task
              </span>
            )
          }
          count={`${todoCompleted}/${todoTotal}`}
          ariaLabel={`Task status, ${todoTotal} ${
            todoTotal === 1 ? "task" : "tasks"
          }`}
        />
        <Capsule
          id="plan"
          open={openId === "plan"}
          expanded={expandedId === "plan"}
          onToggle={toggle}
          onHoverStart={onHoverStart}
          onHoverEnd={onHoverEnd}
          icon={<StrategyIcon size={14} weight="fill" aria-hidden />}
          label="Plan"
          detail={
            activePlanPath ? (
              <span className="truncate">{activePlanPath}</span>
            ) : (
              <span className="italic text-(--_dk-text-disabled)">
                No active plan
              </span>
            )
          }
          ariaLabel="Session plan"
        />
        <Capsule
          id="subagent"
          open={openId === "subagent"}
          expanded={expandedId === "subagent"}
          onToggle={toggle}
          onHoverStart={onHoverStart}
          onHoverEnd={onHoverEnd}
          icon={
            <UsersIcon
              size={14}
              weight="fill"
              aria-hidden
              className={subagentRunning > 0 ? "subagent-status-icon" : ""}
            />
          }
          label="Workers"
          detail={
            subagentTotal > 0 ? (
              <span>
                {subagentRunning}/{subagentTotal} running
              </span>
            ) : (
              <span className="italic text-(--_dk-text-disabled)">
                No subagents
              </span>
            )
          }
          ariaLabel={`Subagent status, ${subagentRunning} running`}
        />
        <Capsule
          id="terminal"
          open={openId === "terminal"}
          expanded={expandedId === "terminal"}
          onToggle={toggle}
          onHoverStart={onHoverStart}
          onHoverEnd={onHoverEnd}
          icon={
            <TerminalIcon
              size={14}
              weight="fill"
              aria-hidden
              className={bashJobs.length > 0 ? "terminal-status-icon" : ""}
            />
          }
          label="Terminals"
          detail={
            bashJobs.length > 0 ? (
              <span className="truncate">
                {bashJobs[bashJobs.length - 1]?.command_preview}
              </span>
            ) : (
              <span className="italic text-(--_dk-text-disabled)">
                No active terminals
              </span>
            )
          }
          count={`×${bashJobs.length}`}
          ariaLabel={`Terminal status, ${bashJobs.length} active`}
        />
      </div>
    </div>
  );
}

/** A single resident capsule. Collapsed it shows only its glyph; expanded —
 * the row-level public slot — it stretches to fill the row (animated width)
 * and shows glyph + label + rich detail + count. Fully controlled: hover and
 * expansion state live in the parent row. The content wrapper stays mounted
 * so collapsing is a pure width+opacity animation (flex reflow cannot
 * transition), hidden with `visibility` when collapsed. */
function Capsule({
  id,
  open,
  expanded,
  onToggle,
  onHoverStart,
  onHoverEnd,
  icon,
  label,
  detail,
  count,
  ariaLabel,
}: {
  id: CapsuleId;
  open: boolean;
  expanded: boolean;
  onToggle: (id: CapsuleId) => void;
  onHoverStart: (id: CapsuleId) => void;
  onHoverEnd: (id: CapsuleId) => void;
  icon: ReactNode;
  label: string;
  /** Rich detail shown only in the expanded (full-row) state. */
  detail: ReactNode;
  count?: ReactNode;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      data-testid={`capsule-${id}`}
      data-open={open}
      data-expanded={expanded}
      aria-expanded={open}
      aria-label={ariaLabel}
      onClick={() => onToggle(id)}
      onMouseEnter={() => onHoverStart(id)}
      onMouseLeave={() => onHoverEnd(id)}
      style={{
        flexBasis: CAPSULE_BASE_PX,
        flexGrow: expanded ? 1 : 0,
        // Inline: composerCardClass ships its own `transition-shadow`, which
        // beats same-specificity transition-* classes in the cascade.
        transitionProperty:
          "flex-grow, color, background-color, border-color, box-shadow",
        transitionDuration: "200ms",
        transitionTimingFunction: "ease-out",
      }}
      className={`${composerCardClass} flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 overflow-hidden px-2.5 text-left text-xs text-(--_dk-text-secondary) active:brightness-90 ${
        open
          ? "border-(--_dk-line-visible) text-(--_dk-text-primary)"
          : "hover:text-(--_dk-text-primary)"
      }`}
    >
      <span className="flex shrink-0 items-center">{icon}</span>
      <span
        data-content-hidden={!expanded}
        className={`flex min-w-0 flex-1 items-center gap-1.5 transition-[opacity,visibility] ${
          expanded
            ? "visible opacity-100 duration-150 delay-150"
            : "invisible opacity-0 duration-100"
        }`}
      >
        <span className="shrink-0 whitespace-nowrap">{label}</span>
        <span className="min-w-0 flex-1 truncate">{detail}</span>
        {count != null && (
          <span className="shrink-0 font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)">
            {count}
          </span>
        )}
      </span>
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

/** Plan panel: renders the active plan file's markdown from the workspace.
 *  The file row ends with the Open affordance; a missing/unreadable file
 *  collapses to a "lost" state instead of dead content. */
type PlanDoc =
  | { status: "loading" }
  | { status: "ok"; md: string }
  | { status: "lost" };

function PlanPanel({
  path,
  projectRoot,
  onOpen,
}: {
  path: string | null;
  projectRoot: string | null;
  onOpen: (path: string) => void;
}) {
  const [doc, setDoc] = useState<PlanDoc>({ status: "loading" });

  useEffect(() => {
    if (!path) return;
    let cancelled = false;
    setDoc({ status: "loading" });
    const resolved = normalizeToolFilePath(path, projectRoot);
    if (!resolved) {
      setDoc({ status: "lost" });
      return;
    }
    void readFile(resolved)
      .then((md) => {
        if (!cancelled) setDoc({ status: "ok", md });
      })
      .catch(() => {
        if (!cancelled) setDoc({ status: "lost" });
      });
    return () => {
      cancelled = true;
    };
  }, [path, projectRoot]);

  if (!path) return <PanelEmpty>No active plan</PanelEmpty>;
  return (
    <div className="flex flex-col gap-2 px-3 py-1">
      <div className="flex items-center gap-2" data-testid="plan-file-row">
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-(--_dk-text-secondary)">
          {path}
        </span>
        <button
          type="button"
          onClick={() => onOpen(path)}
          className="flex shrink-0 items-center gap-1.5 rounded border border-(--_dk-line) px-2 py-1 text-xs text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-primary)"
        >
          <StrategyIcon size={13} weight="fill" aria-hidden />
          Open plan
        </button>
      </div>
      {doc.status === "loading" && (
        <div className="text-xs text-(--_dk-text-disabled)">Loading…</div>
      )}
      {doc.status === "lost" && (
        <div className="text-xs italic text-(--_dk-text-disabled)">lost</div>
      )}
      {doc.status === "ok" && (
        <div className="text-dk-base">
          <AgentMarkdown text={doc.md} />
        </div>
      )}
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
          {items
            // The header above already shows the current in_progress task, so
            // the list skips it — no duplicated first row.
            .filter((item) => item.status !== "in_progress")
            .map((item) => (
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

