import {
  type ClipboardEvent,
  type FormEvent,
  type KeyboardEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";

import type { ContextMode, ThinkingTier } from "../api/types";
import { uploadMedia } from "../api/media";
import {
  MAX_COMPOSER_IMAGES,
  clipboardImageFiles,
  normalizeImage,
} from "../lib/imageNormalize";

import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { subscribeComposerAppend } from "../stores/composerDraft";
import { useToastStore } from "../stores/toastStore";
import { useTurnStore } from "../stores/turnStore";
import { ContextUsageRing } from "./ContextUsageRing";
import {
  Dropdown,
  dropdownItemClass,
  dropdownItemActiveClass,
} from "./ui/Dropdown";
import { ModelSwitcher } from "./ModelSwitcher";
import { NotificationBell } from "./NotificationBell";
import { ShapeBlur } from "./ShapeBlur";
import { ImageThumb } from "./ImageThumb";
import { composerCardClass, actionButtonGlass } from "./composerCard";
import { AgentTypeIcon, agentColor } from "./agentIdentity";

const CTRL_H = "h-7";
const CTRL_TEXT = "text-[11px]";
const PRESS =
  "transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90 disabled:pointer-events-none disabled:opacity-40 disabled:active:scale-100";
const DISABLED_CTRL = "disabled:cursor-not-allowed";
const CTRL_BASE = `${CTRL_H} ${CTRL_TEXT} ${PRESS} ${DISABLED_CTRL} box-border flex items-center rounded-md border border-transparent px-2 leading-none text-(--_dk-text-muted) hover:text-(--_dk-ix-fg-hover)`;
const CTRL_BTN = `${CTRL_BASE} hover:bg-(--_dk-ix-bg-hover)`;

interface ComposerImage {
  id: string;
  phase: "uploading" | "ready";
  ref?: string;
  previewUrl: string;
}

function revokeImages(images: ComposerImage[]) {
  for (const image of images) {
    if (image.previewUrl) URL.revokeObjectURL(image.previewUrl);
  }
}

export function AgentPicker({
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

export function ThinkSlider({
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
                selected
                  ? "text-(--_dk-accent-hover) hover:text-(--_dk-accent-hover)"
                  : ""
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

export function ContextModeToggle({
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
      <span className="relative z-10 whitespace-nowrap">
        {isMax ? "Max" : "Default"}
      </span>
    </button>
  );
}

export function AgentChatInput({
  sessionId,
  variant = "primary",
}: {
  sessionId: string;
  /** `subagent`: the child-session variant. Model / thinking tier / context
   *  mode and the usage ring stay — those are the session-row writes a child
   *  accepts — while every human-composition surface (agent picker, textarea,
   *  send/cancel, notification bell) is absent: a child never takes a user
   *  message, and its agent identity is fixed by its profile. */
  variant?: "primary" | "subagent";
}) {
  const subagentView = variant === "subagent";
  const connection = useConnectionStore((s) => s.state);
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const compacting = useTurnStore(
    (s) => s.byId.get(sessionId)?.compacting ?? false,
  );
  const replaying = useTurnStore(
    (s) => s.byId.get(sessionId)?.replaying ?? false,
  );
  const startAction = useTurnStore((s) => s.start);
  const cancelAction = useTurnStore((s) => s.cancel);
  const enqueueAction = useTurnStore((s) => s.enqueuePending);
  const [draft, setDraft] = useState("");
  const [images, setImages] = useState<ComposerImage[]>([]);
  const imageEpoch = useRef(0);
  useEffect(() => {
    imageEpoch.current += 1;
    setDraft("");
    setImages((current) => {
      revokeImages(current);
      return [];
    });
  }, [sessionId]);

  // Recall hand-off: a queued bubble pulled back from the transcript appends to
  // whatever is already being written — never replaces it — and takes the caret.
  useEffect(() => {
    return subscribeComposerAppend((target, text, recalled = []) => {
      if (target !== sessionId) return;
      setDraft((current) =>
        current.trim() ? `${current.trimEnd()}\n\n${text}` : text,
      );
      if (recalled.length > 0) {
        setImages((current) => [
          ...current,
          ...recalled.map((ref, index) => ({
            id: `recall-${ref}-${current.length + index}`,
            phase: "ready" as const,
            ref,
            previewUrl: "",
          })),
        ]);
      }
      requestAnimationFrame(() => {
        const ta = textareaRef.current;
        if (!ta) return;
        ta.focus();
        ta.setSelectionRange(ta.value.length, ta.value.length);
        ta.style.height = "auto";
        ta.style.height = `${Math.min(ta.scrollHeight, 256)}px`;
      });
    });
  }, [sessionId]);

  const startAgent = (input: string, refs: string[]) =>
    startAction(sessionId, input, false, refs);
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
  const supportsImage =
    availableModels
      .find((model) => model.id === sessionModelId)
      ?.modalities?.includes("image") === true;
  const readyRefs = images.flatMap((image) =>
    image.phase === "ready" && image.ref ? [image.ref] : [],
  );
  const imagesBusy = images.some((image) => image.phase === "uploading");
  const imagesMasked = images.length > 0 && !supportsImage;
  const imagesBlockSend = imagesBusy || imagesMasked;
  const hasBody = draft.trim().length > 0 || readyRefs.length > 0;

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
    if (el.hasPointerCapture?.(e.pointerId))
      el.releasePointerCapture(e.pointerId);
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
  // A session with no stored model is not a dead end: the server repairs it
  // (the agent's model, else the first runnable one) before the turn reads it,
  // so sending is allowed as soon as ANY model is selectable. With no model at
  // all the turn really cannot run — stay blocked.
  const hasModel = Boolean(sessionModelId) || availableModels.length > 0;
  const connBlocked =
    connection !== "connected" || isRunning || compacting || replaying;
  const isBlocked = connBlocked || !hasModel;
  // While a turn is live, the composer queues instead of starting: the server
  // owns the queue (memory-only) and injects it at the next request seam.
  //
  // `compacting` is deliberately not part of this gate: an auto compaction runs
  // *inside* a live turn and the queue is drained at the next seam — i.e. after
  // the compaction — so the message lands in the fresh transcript and the turn
  // keeps stepping to answer it. A standalone compaction (idle session) is
  // still unreachable here: `isRunning` is false, so the send skin stays
  // disabled by `isBlocked`.
  const canQueue =
    isRunning && connection === "connected" && !replaying && hasModel;
  const showQueueAction = canQueue && hasBody;
  // One button, three skins: send while idle, queue while a turn runs and a
  // draft exists, cancel while it runs without one.
  const sendAction: "send" | "queue" | "cancel" = !isRunning
    ? "send"
    : showQueueAction
      ? "queue"
      : "cancel";

  const clearSentImages = (sentIds: Set<string>) => {
    setImages((current) => {
      const keep: ComposerImage[] = [];
      for (const image of current) {
        if (sentIds.has(image.id)) {
          if (image.previewUrl) URL.revokeObjectURL(image.previewUrl);
        } else {
          keep.push(image);
        }
      }
      return keep;
    });
  };

  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    const trimmed = draft.trim();
    if (!hasBody) return;
    if (imagesBusy) return;
    if (imagesMasked) {
      useToastStore
        .getState()
        .showToast("Switch to a model that supports images", "error");
      return;
    }
    const sentIds = new Set(images.map((image) => image.id));
    if (isRunning) {
      if (!canQueue) return;
      void enqueueAction(sessionId, trimmed, readyRefs).then((ok) => {
        if (!ok) return;
        setDraft((current) => (current === draft ? "" : current));
        clearSentImages(sentIds);
      });
      return;
    }
    if (connBlocked) return;
    if (!hasModel) {
      useToastStore
        .getState()
        .showToast("Add a model in Settings first", "error");
      return;
    }
    if (startAgent(trimmed, readyRefs)) {
      setDraft("");
      clearSentImages(sentIds);
    }
  };

  const onPasteImage = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = clipboardImageFiles(event.clipboardData);
    if (files.length === 0) return;
    event.preventDefault();
    const room = MAX_COMPOSER_IMAGES - images.length;
    if (room <= 0) {
      useToastStore
        .getState()
        .showToast(`Up to ${MAX_COMPOSER_IMAGES} images`, "error");
      return;
    }
    const accepted = files.slice(0, room);
    if (accepted.length < files.length) {
      useToastStore
        .getState()
        .showToast(`Up to ${MAX_COMPOSER_IMAGES} images`, "error");
    }
    const epoch = imageEpoch.current;
    for (const file of accepted) {
      const id = `img-${Date.now()}-${Math.random().toString(16).slice(2)}`;
      void (async () => {
        let previewUrl = "";
        try {
          const normalized = await normalizeImage(file);
          if (imageEpoch.current !== epoch) return;
          previewUrl = URL.createObjectURL(normalized);
          setImages((current) => [
            ...current,
            { id, phase: "uploading", previewUrl },
          ]);
          const uploaded = await uploadMedia(normalized);
          if (imageEpoch.current !== epoch) {
            URL.revokeObjectURL(previewUrl);
            return;
          }
          setImages((current) =>
            current.map((image) =>
              image.id === id
                ? { ...image, phase: "ready", ref: uploaded.ref }
                : image,
            ),
          );
        } catch (error) {
          if (previewUrl) URL.revokeObjectURL(previewUrl);
          if (imageEpoch.current !== epoch) return;
          setImages((current) => current.filter((image) => image.id !== id));
          useToastStore
            .getState()
            .showToast(
              error instanceof Error ? error.message : "Could not add the image",
              "error",
            );
        }
      })();
    }
  };

  const removeImage = (id: string) => {
    setImages((current) => {
      const image = current.find((entry) => entry.id === id);
      if (image?.previewUrl) URL.revokeObjectURL(image.previewUrl);
      return current.filter((entry) => entry.id !== id);
    });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (!isBlocked || canQueue) {
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

  if (subagentView) {
    return (
      <div
        data-testid="subagent-controls"
        className={`${composerCardClass} focus-within:border-(--_dk-line-visible)`}
      >
        <div className="flex min-w-0 items-center gap-1 px-1.5 py-1">
          <div className="flex min-w-0 flex-1 items-center gap-1">
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
            {/* Ring next to the context-mode button: with no draft row there is
                no floating action row to host it. Read-only: a child has no
                Compaction action of its own. */}
            <span className="relative ml-0.5 flex h-[30px] w-[30px] shrink-0 items-center justify-center overflow-visible">
              <ShapeBlur
                shape="radial"
                size={34}
                inset={{ left: -2, top: -2 }}
                strength={6}
                maskSolid={40}
                tintColor="var(--_dk-editor)"
                tint={0.66}
              />
              <ContextUsageRing sessionId={sessionId} readOnly />
            </span>
          </div>
        </div>
      </div>
    );
  }

  return (
    <form
      onSubmit={submit}
      className={`${composerCardClass} shrink-0 focus-within:border-(--_dk-line-visible)`}
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
      {images.length > 0 ? (
        <div
          data-testid="composer-images"
          className="flex flex-wrap gap-1.5 px-3 pt-2"
        >
          {images.map((image) =>
            image.phase === "ready" && image.ref ? (
              <ImageThumb
                key={image.id}
                mediaRef={image.ref}
                masked={!supportsImage}
                onRemove={() => removeImage(image.id)}
              />
            ) : (
              <span
                key={image.id}
                className="relative inline-flex max-h-[120px] max-w-[160px] overflow-hidden rounded-md border border-(--_dk-line) bg-(--_dk-editor)"
              >
                {image.previewUrl ? (
                  <img
                    src={image.previewUrl}
                    alt=""
                    className="max-h-[120px] max-w-[160px] object-contain"
                  />
                ) : null}
                {!supportsImage ? (
                  <span className="absolute inset-0 flex items-center justify-center bg-black/55 text-xs text-white">
                    Unsupported
                  </span>
                ) : null}
                <button
                  type="button"
                  aria-label="Remove image"
                  className="absolute right-0.5 top-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-black/60 text-[10px] leading-none text-white"
                  onClick={() => removeImage(image.id)}
                >
                  ×
                </button>
              </span>
            ),
          )}
        </div>
      ) : null}
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
          onPaste={onPasteImage}
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
                ? "Add a model in Settings first..."
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
        {/* Floating action row. Bell (fill = state) and ring (a circle on its
            own) stay bare; only the send/cancel action buttons carry the 66%
            glass fill + blur (actionButtonGlass) so scrolled draft text under
            them is softly obscured instead of colliding. */}
        <div className="absolute right-2 bottom-2 z-10 flex items-center gap-1.5">
          <NotificationBell sessionId={sessionId} />
          {/* Ring container stays ring-sized (30×30); overflow-visible keeps the
              blur halo from being clipped — it spills out as a pure visual
              overlay and never contributes to layout size. */}
          <span className="relative flex h-[30px] w-[30px] items-center justify-center overflow-visible">
            {/* EXPERIMENTAL ShapeBlur test — sits under the ring (DOM order).
                Delete or move freely; does not touch ProgressiveBlur. */}
            <ShapeBlur
              shape="radial"
              size={34}
              inset={{ left: -2, top: -2 }}
              strength={6}
              maskSolid={40}
              tintColor="var(--_dk-editor)"
              tint={0.66}
            />
            <ContextUsageRing sessionId={sessionId} />
          </span>
          {/* One button, three skins. The glyphs share a single grid cell and
              cross-scale on the spot: the outgoing icon shrinks away while the
              incoming one grows in, so a state change reads as one gesture
              instead of an instant glyph swap. `sendAction` owns the behaviour
              (type / disabled / onClick / title), the stacked layers own the
              look. The box never remounts, so Enter's press feedback and focus
              survive a swap; both live-turn skins keep the breathing edge glow. */}
          <button
            ref={sendBtnRef}
            type={sendAction === "send" ? "submit" : "button"}
            disabled={
              (sendAction === "send" &&
                (isBlocked || !hasBody || imagesBlockSend)) ||
              (sendAction === "queue" && imagesBlockSend)
            }
            onClick={
              sendAction === "send"
                ? undefined
                : sendAction === "queue"
                  ? () => submit()
                  : cancelAgent
            }
            className={`${actionButtonGlass} flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-md border border-(--_dk-border-strong) text-(--_dk-text-primary) transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90 disabled:cursor-not-allowed disabled:opacity-40 disabled:brightness-100${
              isRunning ? " send-spin-glow" : ""
            }`}
            title={
              sendAction !== "cancel" && imagesMasked
                ? "Switch to a model that supports images"
                : sendAction !== "cancel" && imagesBusy
                  ? "Waiting for the image to upload"
                  : sendAction === "send"
                    ? "Send"
                    : sendAction === "queue"
                      ? "Queue for the next step"
                      : "Cancel"
            }
          >
            <span className="composer-action-icons">
              <svg
                data-on={sendAction === "send"}
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M5 12h14M12 5l7 7-7 7" />
              </svg>
              {/* The same arrow turned 90° counter-clockwise (up): a queued
                  message waits for the next request seam, it does not send. */}
              <svg
                data-on={sendAction === "queue"}
                className="-rotate-90"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M5 12h14M12 5l7 7-7 7" />
              </svg>
              {/* The cancel skin carries a pair of its own (stop square ↔ the
                  cancelling swirl), stacked the same way inside it, so the
                  spinner grows in as the square shrinks away. */}
              <span
                data-on={sendAction === "cancel"}
                className="composer-action-icons"
              >
                <svg
                  data-on={runState === "cancelling"}
                  className="h-4 w-4 animate-spin"
                  viewBox="0 0 24 24"
                  fill="none"
                >
                  <circle
                    cx="12"
                    cy="12"
                    r="10"
                    stroke="currentColor"
                    strokeWidth="3"
                    opacity="0.25"
                  />
                  <path
                    d="M12 2a10 10 0 0 1 10 10"
                    stroke="currentColor"
                    strokeWidth="3"
                    strokeLinecap="round"
                  />
                </svg>
                <svg
                  data-on={runState !== "cancelling"}
                  width="12"
                  height="12"
                  viewBox="0 0 12 12"
                  fill="currentColor"
                >
                  <rect x="1" y="1" width="10" height="10" rx="1.5" />
                </svg>
              </span>
            </span>
          </button>
        </div>
      </div>
    </form>
  );
}
