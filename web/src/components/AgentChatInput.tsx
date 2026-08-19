import { type FormEvent, type KeyboardEvent, useEffect, useRef, useState } from "react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";

import type { ContextMode, ThinkingTier } from "../api/types";

import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { useTurnStore } from "../stores/turnStore";
import { ContextUsageRing } from "./ContextUsageRing";
import { Dropdown, dropdownItemClass, dropdownItemActiveClass } from "./ui/Dropdown";
import { ModelSwitcher } from "./ModelSwitcher";
import { NotificationBell } from "./NotificationBell";
import { composerCardClass } from "./composerCard";
import { AgentTypeIcon, agentColor } from "./agentIdentity";

const CTRL_H = "h-7";
const CTRL_TEXT = "text-[11px]";
const PRESS =
  "transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90 disabled:pointer-events-none disabled:opacity-40 disabled:active:scale-100";
const DISABLED_CTRL = "disabled:cursor-not-allowed";
const CTRL_BASE = `${CTRL_H} ${CTRL_TEXT} ${PRESS} ${DISABLED_CTRL} box-border flex items-center rounded-md border border-transparent px-2 leading-none text-(--_dk-text-muted) hover:text-(--_dk-ix-fg-hover)`;
const CTRL_BTN = `${CTRL_BASE} hover:bg-(--_dk-ix-bg-hover)`;

function AgentPicker({
  agents,
  activeId,
  pendingId,
  disabled,
  onChange,
}: {
  agents: { id: string }[];
  activeId: string;
  pendingId: string | null;
  disabled: boolean;
  onChange: (id: string) => void;
}) {
  const currentId = pendingId ?? activeId;
  const color = agentColor(currentId);

  return (
    <Dropdown
      direction="up"
      variant="select"
      className="min-w-[32px] max-w-[150px] shrink"
      panelClassName="rounded-md"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          disabled={disabled}
          onClick={toggle}
          className={`${CTRL_BTN} w-full min-w-0 justify-center gap-1 ${
            open ? "bg-(--_dk-ix-bg-hover)" : ""
          }`}
          title={currentId}
        >
          <span className="shrink-0">
            <AgentTypeIcon role="primary" color={color} />
          </span>
          <span className="min-w-0 truncate">{currentId}</span>
        </button>
      )}
    >
      {agents.map((a) => {
        const c = agentColor(a.id);
        const isActive = a.id === currentId;
        return (
          <button
            key={a.id}
            type="button"
            onClick={() => onChange(a.id)}
            className={`${dropdownItemClass} ${PRESS} flex items-center gap-1.5 ${isActive ? dropdownItemActiveClass : ""}`}
          >
            <AgentTypeIcon role="primary" color={c} />
            {a.id}
          </button>
        );
      })}
    </Dropdown>
  );
}

function ThinkSlider({
  sessionId,
  value,
  disabled,
  onChange,
}: {
  sessionId: string;
  value: ThinkingTier;
  disabled: boolean;
  onChange: (v: ThinkingTier) => void;
}) {
  const reduceMotion = useReducedMotion();
  const segments: { label: string; tier: ThinkingTier }[] = [
    { label: "Low", tier: "low" },
    { label: "Med", tier: "medium" },
    { label: "High", tier: "high" },
  ];
  return (
    <LayoutGroup id={`think-${sessionId}`}>
    <div className="flex w-[124px] shrink-0 justify-between">
      {segments.map(({ label, tier }) => {
        const selected = value === tier;
        return (
          <button
            key={tier}
            type="button"
            disabled={disabled}
            onClick={() => onChange(tier)}
            className={`group ${CTRL_BASE} relative ${
              selected ? "text-(--_dk-accent-hover) hover:text-(--_dk-accent-hover)" : ""
            }`}
          >
            {selected ? (
              <motion.span
                layoutId={`think-pill-${sessionId}`}
                className="absolute inset-0 rounded-md bg-(--_dk-accent-halo)"
                transition={
                  reduceMotion
                    ? { duration: 0 }
                    : { type: "spring", stiffness: 420, damping: 34 }
                }
              />
            ) : (
              <span className="pointer-events-none absolute inset-0 rounded-md border border-transparent group-hover:border-(--_dk-line)" />
            )}
            <span className="relative z-10">{label}</span>
          </button>
        );
      })}
    </div>
    </LayoutGroup>
  );
}

function ContextModeToggle({
  mode,
  disabled,
  onChange,
}: {
  mode: ContextMode;
  disabled: boolean;
  onChange: (mode: ContextMode) => void;
}) {
  const isMax = mode === "max";
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={() => onChange(isMax ? "standard" : "max")}
      className={`${CTRL_BASE} relative w-[64px] shrink-0 justify-center ${
        isMax
          ? "text-(--_dk-accent-hover) hover:text-(--_dk-accent-hover)"
          : "hover:bg-(--_dk-ix-bg-hover)"
      }`}
      title={isMax ? "Context: Max (1M)" : "Context: Default"}
    >
      {isMax ? (
        <span className="absolute inset-0 rounded-md bg-(--_dk-accent-halo)" />
      ) : null}
      <span className="relative z-10 whitespace-nowrap">{isMax ? "Max" : "Default"}</span>
    </button>
  );
}

export function AgentChatInput({ sessionId }: { sessionId: string }) {
  const connection = useConnectionStore((s) => s.state);
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const compacting = useTurnStore(
    (s) => s.byId.get(sessionId)?.compacting ?? false,
  );
  const startAction = useTurnStore((s) => s.start);
  const cancelAction = useTurnStore((s) => s.cancel);
  const [draft, setDraft] = useState("");
  useEffect(() => {
    setDraft("");
  }, [sessionId]);

  const startAgent = (input: string) => startAction(sessionId, input);
  const cancelAgent = () => {
    cancelAction(sessionId);
  };
  const setThinkingTier = useSessionStore((s) => s.setThinkingTier);
  const setContextMode = useSessionStore((s) => s.setContextMode);
  const thinkingTier = useSessionStore((s) => {
    const slice = s.byId.get(sessionId);
    return slice?.pendingThinkingTier ?? slice?.thinkingTier ?? "medium";
  });
  const contextMode = useSessionStore((s) => {
    const slice = s.byId.get(sessionId);
    return slice?.pendingContextMode ?? slice?.contextMode ?? "standard";
  });
  const activePrimary = useSessionStore((s) => {
    const slice = s.byId.get(sessionId);
    return slice?.activePrimary ?? s.activePrimary;
  });
  const primaryAgents = useSessionStore((s) => s.primaryAgents);
  const pendingPrimaryId = useSessionStore((s) => {
    const slice = s.byId.get(sessionId);
    return slice?.pendingPrimaryId ?? null;
  });
  const setActivePrimary = useSessionStore((s) => s.setPrimary);
  const sessionModelId = useSessionStore(
    (s) => s.byId.get(sessionId)?.modelId ?? null,
  );
  const availableModels = useSessionStore((s) => s.availableModels);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const sendBtnRef = useRef<HTMLButtonElement>(null);
  const draggingRef = useRef(false);
  const dragStartRef = useRef({ y: 0, h: 0 });
  const rafRef = useRef<number | null>(null);

  // Cancel any pending resize frame on unmount to avoid writing to a
  // detached textarea.
  useEffect(() => {
    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  const applyResize = (clientY: number) => {
    rafRef.current = null;
    const ta = textareaRef.current;
    if (!ta) return;
    const deltaY = dragStartRef.current.y - clientY;
    const newH = Math.max(36, dragStartRef.current.h + deltaY);
    ta.style.height = `${newH}px`;
  };

  // Pointer Events + setPointerCapture: once captured, every subsequent
  // pointermove/pointerup for this pointer is delivered to the handle — even
  // if the cursor leaves the element, the window, or moves over an iframe.
  // This is what makes the drag robust (no more "drag a few px then snap back").
  const onResizeStart = (e: React.PointerEvent) => {
    e.preventDefault();
    const ta = textareaRef.current;
    if (!ta) return;
    draggingRef.current = true;
    dragStartRef.current = { y: e.clientY, h: ta.offsetHeight };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    document.body.style.userSelect = "none";
    document.body.style.cursor = "ns-resize";
  };

  const onResizeEnd = (e: React.PointerEvent) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture?.(e.pointerId)) el.releasePointerCapture(e.pointerId);
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
  };

  const onResizeMove = (e: React.PointerEvent) => {
    if (!draggingRef.current) return;
    // Coalesce high-frequency move events into a single style write per frame.
    const y = e.clientY;
    if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => applyResize(y));
  };

  const isRunning = runState === "running" || runState === "cancelling";
  const hasModel = Boolean(sessionModelId);
  const connBlocked = connection !== "connected" || isRunning || compacting;
  const isBlocked = connBlocked || !hasModel;

  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (connBlocked) return;
    if (!hasModel) {
      useToastStore
        .getState()
        .showToast(
          availableModels.length === 0
            ? "Add a model in Settings first"
            : "Select a model before sending",
          "error",
        );
      return;
    }
    const trimmed = draft.trim();
    if (!trimmed) return;
    if (startAgent(draft)) {
      setDraft("");
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (!isBlocked) {
        // Keyboard submit has no :active, so fire press feedback manually.
        const btn = sendBtnRef.current;
        if (btn) {
          btn.classList.remove("send-press");
          void btn.offsetWidth; // force reflow to restart animation
          btn.classList.add("send-press");
        }
        submit();
      }
    }
  };

  return (
    <form
      onSubmit={submit}
      className={`${composerCardClass} focus-within:border-(--_dk-line-visible)`}
    >
      <div className="flex min-w-0 items-center gap-1 overflow-hidden px-1.5 py-1">
        <div className="flex min-w-0 flex-1 items-center gap-1">
          {primaryAgents.length > 0 && (
            <AgentPicker
              agents={primaryAgents}
              activeId={activePrimary}
              pendingId={pendingPrimaryId}
              disabled={connBlocked}
              onChange={(id: string) => {
                setActivePrimary(sessionId, id);
              }}
            />
          )}
          <ModelSwitcher sessionId={sessionId} disabled={connBlocked} />
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <ThinkSlider
            sessionId={sessionId}
            value={thinkingTier}
            disabled={connBlocked}
            onChange={(tier) => setThinkingTier(sessionId, tier)}
          />
          <div className="mx-0.5 h-3.5 w-px shrink-0 bg-(--_dk-line)" />
          <ContextModeToggle
            mode={contextMode}
            disabled={connBlocked}
            onChange={(mode) => setContextMode(sessionId, mode)}
          />
        </div>
      </div>
      <div className="mx-3 border-t border-(--_dk-line)" />
      <div className="relative overflow-hidden rounded-b-[calc(var(--radius-sm)-1px)]">
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            const ta = textareaRef.current;
            if (ta) {
              ta.style.height = "auto";
              ta.style.height = `${Math.min(ta.scrollHeight, 256)}px`;
            }
          }}
          onKeyDown={onKeyDown}
          onDragOver={(e) => {
            if (e.dataTransfer.types.includes("text/plain")) e.preventDefault();
          }}
          onDrop={(e) => {
            e.preventDefault();
            const text = e.dataTransfer.getData("text/plain");
            if (!text) return;
            const ta = textareaRef.current;
            if (!ta) {
              setDraft((d) => (d ? `${d}\n${text}` : text));
              return;
            }
            const start = ta.selectionStart;
            const end = ta.selectionEnd;
            setDraft((d) => d.slice(0, start) + text + d.slice(end));
          }}
          placeholder={
            connection !== "connected"
              ? "Waiting for connection..."
              : !hasModel
                ? availableModels.length === 0
                  ? "Add a model in Settings first..."
                  : "Select a model above, then message the agent..."
                : "Message the agent..."
          }
          // disabled={isBlocked} — never disable, just block Enter key
          rows={3}
          className="w-full resize-none border-0 bg-transparent px-3 pt-2 pb-11 text-sm max-h-48 text-(--_dk-text-primary) outline-none placeholder:text-(--_dk-text-disabled) focus-visible:shadow-none disabled:cursor-not-allowed disabled:opacity-50"
        />
        {/* Top-right drag handle */}
        <div
          className="absolute right-0.5 top-0 flex h-4 w-6 cursor-ns-resize items-center justify-center text-(--_dk-text-disabled) select-none"
          style={{ touchAction: "none" }}
          onPointerDown={onResizeStart}
          onPointerMove={onResizeMove}
          onPointerUp={onResizeEnd}
          onPointerCancel={onResizeEnd}
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
            <circle cx="2" cy="2" r="1" />
            <circle cx="5" cy="2" r="1" />
            <circle cx="8" cy="2" r="1" />
          </svg>
        </div>
        <div className="absolute right-2 bottom-2 z-10 flex items-center gap-1.5">
          <NotificationBell sessionId={sessionId} />
          <ContextUsageRing sessionId={sessionId} />
          {isRunning ? (
            <button
              type="button"
              onClick={cancelAgent}
              className="send-spin-glow flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-md border border-(--_dk-border-strong) bg-transparent text-(--_dk-text-primary) transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90"
              title="Cancel"
            >
              {runState === "cancelling" ? (
                <svg className="h-4 w-4 animate-spin" viewBox="0 0 24 24" fill="none">
                  <circle cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" opacity="0.25" />
                  <path d="M12 2a10 10 0 0 1 10 10" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
                </svg>
              ) : (
                <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
                  <rect x="1" y="1" width="10" height="10" rx="1.5" />
                </svg>
              )}
            </button>
          ) : (
            <button
              ref={sendBtnRef}
              type="submit"
              disabled={isBlocked || !draft.trim()}
              className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-md border border-(--_dk-border-strong) bg-transparent text-(--_dk-text-primary) transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90 disabled:cursor-not-allowed disabled:opacity-40 disabled:brightness-100"
              title="Send"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M5 12h14M12 5l7 7-7 7" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </form>
  );
}
