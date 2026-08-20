import { create } from "zustand";

import { applyTurnEventMeta, isCompactCutRow, isUserMessage, itemPlainText, newPendingUserId, userTextItem } from "../api/adapter";
import type {
  AgentRunState,
  TurnPhase,
  TurnSnapshot,
  TurnStarted,
  TurnEventEnvelope,
  TurnFinished,
  PermissionRequest,
  ResponseStreamEvent,
  SessionSnapshot,
  CompactLifecycle,
} from "../api/types";
import { debugTrace } from "../lib/debugTrace";
import { useConnectionStore, attachSiblingStores } from "./connectionStore";
import { useMessageStore, type TurnEndNotice } from "./messageStore";
import { useNotificationStore } from "./notificationStore";
import { toastLlmConfigFailure, useSettingsStore } from "./settingsStore";
import { useToastStore } from "./toastStore";

export interface PendingPermission {
  turn_id: string;
  request_id: string;
  tool: string;
  rule_id: string;
  summary: string;
}

/** Per-session turn runtime state only — no composer/UI draft fields. */
export interface TurnSlice {
  runState: AgentRunState;
  currentTurnId: string | null;
  pendingCancel: boolean;
  pendingPermission: PendingPermission | null;
  turnPhase: TurnPhase | null;
  turnStep: number | null;
  turnStepMax: number | null;
  contextWindow: number;
  contextTokensEstimate: number;
  compactEligible: boolean;
  compacting: boolean;
  /** Provider last-request prompt_tokens (truth). 0 = absent. */
  lastTurnPromptTokens: number;
  lastTurnCompletionTokens: number;
  lastTurnCacheHitTokens: number;
  lastTurnCacheMissTokens: number;
  /** Session-total provider usage (Σ every request; whole-session hit rate). */
  sessionPromptTokens: number;
  sessionCompletionTokens: number;
  sessionCacheHitTokens: number;
  sessionCacheMissTokens: number;
  stopReason: string | null;
  todoPending: number;
  todoInProgress: number;
  todoCompleted: number;
  todoItems: { id: string; content: string; status: "pending" | "in_progress" | "completed" }[];
}

export function emptySlice(): TurnSlice {
  return { ...EMPTY_SLICE };
}

export const EMPTY_SLICE: TurnSlice = {
  runState: "idle",
  currentTurnId: null,
  pendingCancel: false,
  pendingPermission: null,
  turnPhase: null,
  turnStep: null,
  turnStepMax: null,
  contextWindow: 0,
  contextTokensEstimate: 0,
  compactEligible: false,
  compacting: false,
  lastTurnPromptTokens: 0,
  lastTurnCompletionTokens: 0,
  lastTurnCacheHitTokens: 0,
  lastTurnCacheMissTokens: 0,
  sessionPromptTokens: 0,
  sessionCompletionTokens: 0,
  sessionCacheHitTokens: 0,
  sessionCacheMissTokens: 0,
  stopReason: null,
  todoPending: 0,
  todoInProgress: 0,
  todoCompleted: 0,
  todoItems: [],
};

function todoPatchFromItems(
  items: {
    id: string;
    content: string;
    status: "pending" | "in_progress" | "completed";
  }[],
): Pick<
  TurnSlice,
  "todoPending" | "todoInProgress" | "todoCompleted" | "todoItems"
> {
  const mapped = items.map((i) => ({
    id: i.id,
    content: i.content,
    status: i.status,
  }));
  return {
    todoItems: mapped,
    todoPending: mapped.filter((t) => t.status === "pending").length,
    todoInProgress: mapped.filter((t) => t.status === "in_progress").length,
    todoCompleted: mapped.filter((t) => t.status === "completed").length,
  };
}

/**
 * Todo patch that keeps a struck-through history when the backend clears the
 * list (empty items) but we previously had entries: the expanded list keeps
 * the old rows as completed, while the ring/counts follow the backend (0/0/0)
 * so the ring clears instead of showing all-green. A genuinely empty list
 * (never had items) and any non-empty list pass straight through.
 */
function todoPatchRetaining(
  prev: TurnSlice,
  items: {
    id: string;
    content: string;
    status: "pending" | "in_progress" | "completed";
  }[],
): Pick<
  TurnSlice,
  "todoPending" | "todoInProgress" | "todoCompleted" | "todoItems"
> {
  if (items.length === 0 && prev.todoItems.length > 0) {
    return {
      todoItems: prev.todoItems.map((i) => ({
        ...i,
        status: "completed" as const,
      })),
      todoPending: 0,
      todoInProgress: 0,
      todoCompleted: 0,
    };
  }
  return todoPatchFromItems(items);
}

function getSlice(byId: Map<string, TurnSlice>, sessionId: string): TurnSlice {
  let slice = byId.get(sessionId);
  if (!slice) {
    slice = emptySlice();
    byId.set(sessionId, slice);
  }
  return slice;
}

/**
 * Whether this end event owns the live turn. Duplicate idle finishes and
 * finishes for a previous turn (next turn already started) must not idle
 * the panel or touch the overlay.
 */
export function shouldApplyTurnEnd(
  currentTurnId: string | null,
  runState: AgentRunState,
  finishedTurnId?: string | null,
): boolean {
  if (runState === "idle" && !currentTurnId) return false;
  if (!currentTurnId && (runState === "running" || runState === "cancelling")) {
    return false;
  }
  if (finishedTurnId && currentTurnId && finishedTurnId !== currentTurnId) {
    return false;
  }
  return true;
}

function turnEndNoticeFrom(tf: TurnFinished): TurnEndNotice | null {
  if (tf.reason === "error") {
    return {
      kind: "error",
      message: tf.error?.message ?? tf.final_text ?? "Turn failed",
    };
  }
  if (tf.reason === "max_steps") {
    return { kind: "error", message: "Max steps reached" };
  }
  if (tf.reason === "hook_blocked") {
    return {
      kind: "error",
      message: tf.error?.message ?? "Blocked by hook",
    };
  }
  return null;
}

interface TurnStore {
  byId: Map<string, TurnSlice>;

  start: (sessionId: string, input: string) => boolean;
  compact: (sessionId: string) => void;
  cancel: (sessionId: string) => void;
  grantPermission: (sessionId: string, approved: boolean, always: boolean) => void;

  onPermissionRequest: (sessionId: string, pr: PermissionRequest) => void;
  onTurnStarted: (ts: TurnStarted) => void;
  onTurnEvent: (te: TurnEventEnvelope) => void;
  onTurnFinished: (tf: TurnFinished) => void;
  /** List-channel turn end; idle only if this still owns the live turn. */
  onLifecycleTurnFinished: (
    sessionId: string,
    finishedTurnId?: string | null,
  ) => void;
  /** Transcript revert cancelled the live turn; wait for turn_finished. */
  onTranscriptReverted: (sessionId: string) => void;
  onCompactLifecycle: (life: CompactLifecycle) => void;
  applySnapshotTurn: (sessionId: string, turn: TurnSnapshot | null | undefined) => void;
  /** Hydrate context ring fields from a session snapshot (subscribe / reload). */
  applySnapshotMeter: (sessionId: string, snap: SessionSnapshot) => void;
  resetTurn: (sessionId: string) => void;
  /** Flush rAF-coalesced stream deltas before authority seal. */
  flushPendingStream: (sessionId: string) => void;
  /** Drop queued stream deltas without applying them (transcript revert). */
  clearPendingStream: (sessionId: string) => void;
}

function deriveRunState(turn: TurnSnapshot | null | undefined): AgentRunState {
  if (!turn) return "idle";
  const { phase } = turn;
  if (phase === "idle") return "idle";
  if (typeof phase === "object" && phase !== null && "failed" in phase) {
    return "idle";
  }
  if (phase === "cancelling") return "cancelling";
  return "running";
}

// Coalesce stream deltas so the message list re-renders at most once per
// animation frame instead of once per token. Each WS delta currently triggers
// a full re-render of the (virtualized) list, which is the dominant cost during
// streaming. Buffering and flushing on rAF caps that to ~60 renders/sec and lets
// the browser paint between batches.
interface PendingStream {
  turnId: string;
  events: ResponseStreamEvent[];
}
const pendingStreamBySession = new Map<string, PendingStream>();
const rafBySession = new Map<string, number>();

function flushStreamSession(sessionId: string): void {
  const raf = rafBySession.get(sessionId);
  if (raf !== undefined) {
    cancelAnimationFrame(raf);
    rafBySession.delete(sessionId);
  }
  const pending = pendingStreamBySession.get(sessionId);
  if (!pending) return;
  pendingStreamBySession.delete(sessionId);
  const msg = useMessageStore.getState();
  for (const ev of pending.events) {
    msg.applyStreamEvent(sessionId, pending.turnId, 0, ev);
  }
}

/**
 * Drain rAF-buffered stream deltas for a session.
 * Must run before `buffer/item` seal so authority text is not followed by
 * late appends (DeepSeek/chat path has no output_text.done to repair this).
 */
export function flushPendingStream(sessionId: string): void {
  flushStreamSession(sessionId);
}

function enqueueStreamEvent(
  sessionId: string,
  turnId: string,
  event: ResponseStreamEvent,
): void {
  const existing = pendingStreamBySession.get(sessionId);
  if (existing) {
    existing.events.push(event);
  } else {
    pendingStreamBySession.set(sessionId, { turnId, events: [event] });
  }
  if (rafBySession.has(sessionId)) return;
  const raf = requestAnimationFrame(() => {
    rafBySession.delete(sessionId);
    flushStreamSession(sessionId);
  });
  rafBySession.set(sessionId, raf);
}

function clearStreamSession(sessionId: string): void {
  const raf = rafBySession.get(sessionId);
  if (raf !== undefined) {
    cancelAnimationFrame(raf);
    rafBySession.delete(sessionId);
  }
  pendingStreamBySession.delete(sessionId);
}

export const useTurnStore = create<TurnStore>((set, get) => {
  function patch(
    sessionId: string,
    update: Partial<TurnSlice>,
  ): void {
    const byId = new Map(get().byId);
    const next = { ...getSlice(byId, sessionId), ...update };
    byId.set(sessionId, next);
    set({ byId });
  }

  return {
    byId: new Map(),

    start: (sessionId, input) => {
      const trimmed = input.trim();
      if (!trimmed) return false;

      const ws = useConnectionStore.getState();
      const current = getSlice(get().byId, sessionId);
      if (!ws.sendRpc || current.runState !== "idle") {
        debugTrace("turn", "start.rejected", {
          sessionId,
          runState: current.runState,
          currentTurnId: current.currentTurnId,
        });
        return false;
      }

      const pending = { clientId: newPendingUserId(), item: userTextItem(trimmed) };
      useMessageStore.getState().pushPendingUser(sessionId, pending);

      patch(sessionId, {
        runState: "running",
        currentTurnId: null,
      });
      debugTrace("turn", "start", { sessionId });

      const startPayload = { input: trimmed, session_id: sessionId };

      // Use send (fire-and-forget) for agent/run
      useConnectionStore.getState().sendRpc("agent/run", startPayload).catch((error: unknown) => {
        patch(sessionId, { runState: "idle", currentTurnId: null });
        useMessageStore.getState().discardOptimisticUserMessage(sessionId, pending.clientId);
        const message =
          error instanceof Error ? error.message : "Failed to start agent turn";
        // Config / setup gaps → corner toast with full guidance (not the bell).
        if (
          useSettingsStore.getState().summary?.setup_guidance ||
          /model_ref|no model configured|provider|not found|Settings/i.test(message)
        ) {
          toastLlmConfigFailure(message);
        } else {
          useToastStore.getState().showToast(message, "error", 8000);
        }
      });
      return true;
    },

    compact: (sessionId) => {
      const current = getSlice(get().byId, sessionId);
      if (
        current.runState !== "idle" ||
        current.compacting ||
        !current.compactEligible
      ) {
        return;
      }
      patch(sessionId, { compacting: true });
      useConnectionStore
        .getState()
        .sendRpc("session/compact", { session_id: sessionId })
        .catch((error: unknown) => {
          patch(sessionId, { compacting: false });
          useToastStore.getState().showToast(
            error instanceof Error ? error.message : "Compact failed",
            "error",
          );
        });
    },

    cancel: (sessionId) => {
      const current = getSlice(get().byId, sessionId);
      if (current.runState !== "running") return;
      patch(sessionId, { runState: "cancelling", pendingCancel: true, pendingPermission: null });
      useConnectionStore.getState().sendRpc("agent/cancel", { session_id: sessionId }).catch(() => {});
    },

    onTranscriptReverted: (sessionId) => {
      const current = getSlice(get().byId, sessionId);
      if (current.runState !== "running" && current.runState !== "cancelling") {
        return;
      }
      patch(sessionId, {
        runState: "cancelling",
        pendingCancel: true,
        pendingPermission: null,
      });
    },

    grantPermission: (sessionId, approved, always) => {
      const current = getSlice(get().byId, sessionId);
      if (!current.pendingPermission) return;
      const pending = current.pendingPermission;

      // Keep the card open until the server's `agent/permission` receipt
      // arrives; only then close it. On failure, restore the card and surface
      // the error explicitly instead of a silent empty catch (FE-04).
      useConnectionStore.getState().sendRpc("agent/permission", {
        request_id: pending.request_id,
        tool: pending.tool,
        approved,
        always,
        session_id: sessionId,
      }).then(() => {
        // Close only the card we granted; a newer request may have replaced it.
        const latest = getSlice(get().byId, sessionId).pendingPermission;
        if (latest?.request_id === pending.request_id) {
          patch(sessionId, { pendingPermission: null });
        }
      }).catch((error: unknown) => {
        // Rollback: keep/restore this card (unless a newer request is showing).
        const latest = getSlice(get().byId, sessionId).pendingPermission;
        if (latest?.request_id === pending.request_id || !latest) {
          patch(sessionId, { pendingPermission: pending });
        }
        const message =
          error instanceof Error
            ? error.message
            : "Permission grant failed";
        useToastStore.getState().showToast(message, "error");
      });
    },

    onPermissionRequest: (sessionId, pr) => {
      // Always is rule-scoped on the server (`agent, tool, rule_id`); never
      // auto-approve by whole-tool name on the client.
      patch(sessionId, {
        pendingPermission: {
          turn_id: pr.turn_id,
          request_id: pr.request_id,
          tool: pr.tool,
          rule_id: pr.rule_id,
          summary: pr.summary,
        },
      });
    },

    onTurnStarted: (ts) => {
      const sessionId = ts.session_id;
      useMessageStore.getState().allowLogGrowth(sessionId);
      debugTrace("turn", "started", {
        sessionId,
        turnId: ts.turn_id,
        stepMax: ts.step_max,
      });

      patch(sessionId, {
        runState: "running",
        currentTurnId: ts.turn_id,
        turnPhase: "starting",
        turnStep: null,
        turnStepMax: ts.step_max,
        pendingPermission: null,
        pendingCancel: false,
      });

      // Server-initiated turns (idle bash auto-turn) never went through `start()`,
      // so there is no optimistic user row. Surface the turn input so the panel
      // matches a normal agent/run. `buffer/item` later seals this `user-*` row.
      const trimmed = (ts.input ?? "").trim();
      if (trimmed) {
        const rows =
          useMessageStore.getState().bySession.get(sessionId)?.messages ?? [];
        const lastUser = [...rows].reverse().find(
          (m) => !isCompactCutRow(m) && isUserMessage(m.item),
        );
        if (!lastUser || itemPlainText(lastUser.item) !== trimmed) {
          useMessageStore.getState().pushPendingUser(sessionId, {
            clientId: newPendingUserId(),
            item: userTextItem(trimmed),
          });
        }
      }
    },

    onTurnEvent: (te) => {
      const sessionId = te.session_id;
      const current = getSlice(get().byId, sessionId);
      if (te.turn_id !== current.currentTurnId) {
        if (te.event.type !== "stream_event") {
          debugTrace("turn", "event.dropped", {
            sessionId,
            eventTurn: te.turn_id,
            currentTurnId: current.currentTurnId,
            type: te.event.type,
          });
        }
        return;
      }

      // Informational turn events → bell. Failures / warnings → corner toast.
      const notify = useNotificationStore.getState();
      switch (te.event.type) {
        case "compaction":
          notify.add(sessionId, "Context compacted");
          break;
        case "hook_fired":
          notify.add(sessionId, `Hook: ${te.event.action}`);
          break;
        case "error":
          if (
            useSettingsStore.getState().summary?.setup_guidance ||
            /model_ref|no model configured|compaction|Settings → Agents|Settings -> Agents/i.test(
              te.event.message,
            )
          ) {
            toastLlmConfigFailure(te.event.message);
          } else {
            useToastStore.getState().showToast(te.event.message, "error", 8000);
          }
          break;
        case "snapshot_notice": {
          const level = te.event.level.toLowerCase();
          const variant = level === "error" || level === "warn" || level === "warning"
            ? "error"
            : "info";
          useToastStore.getState().showToast(te.event.message, variant, 8000);
          break;
        }
        case "permission_resolved":
          notify.add(
            sessionId,
            te.event.approved ? `Approved: ${te.event.tool}` : `Denied: ${te.event.tool}`,
            te.event.approved ? "success" : "error",
          );
          break;
        case "todo_progress":
          patch(sessionId, todoPatchRetaining(current, te.event.items ?? []));
          return;
      }

      // Stream tokens go to messageStore via rAF; do not set turnStore (no
      // meta change) so Todo/Ring/Input/List parents do not re-render per token.
      if (te.event.type === "stream_event") {
        if (current.runState === "running" && current.currentTurnId) {
          enqueueStreamEvent(sessionId, te.turn_id, te.event.event);
        }
        return;
      }

      const updated = new Map(get().byId);
      const slice = { ...getSlice(updated, sessionId) };

      const metaUpdate = applyTurnEventMeta(te.event);
      const mapped: Partial<TurnSlice> = {
        runState: slice.pendingCancel ? "cancelling" : "running",
      };
      if (metaUpdate.phase !== undefined) mapped.turnPhase = metaUpdate.phase;
      if (metaUpdate.step !== undefined) mapped.turnStep = metaUpdate.step;
      if (metaUpdate.stepMax !== undefined) mapped.turnStepMax = metaUpdate.stepMax;
      if (metaUpdate.contextWindow !== undefined) mapped.contextWindow = metaUpdate.contextWindow;
      if (metaUpdate.promptTokens !== undefined) mapped.lastTurnPromptTokens = metaUpdate.promptTokens;
      if (metaUpdate.completionTokens !== undefined) mapped.lastTurnCompletionTokens = metaUpdate.completionTokens;
      if (metaUpdate.cacheHitTokens !== undefined) mapped.lastTurnCacheHitTokens = metaUpdate.cacheHitTokens;
      if (metaUpdate.cacheMissTokens !== undefined) mapped.lastTurnCacheMissTokens = metaUpdate.cacheMissTokens;
      if (metaUpdate.stopReason !== undefined) mapped.stopReason = metaUpdate.stopReason;
      // Session-total accumulators: add each request's usage to the running sum.
      // Snapshot hydrates (applySnapshotMeter / onTurnFinished) overwrite with the
      // authoritative backend value; events only add the increment after that.
      if (metaUpdate.promptTokens !== undefined) {
        mapped.sessionPromptTokens = (slice.sessionPromptTokens ?? 0) + metaUpdate.promptTokens;
      }
      if (metaUpdate.completionTokens !== undefined) {
        mapped.sessionCompletionTokens =
          (slice.sessionCompletionTokens ?? 0) + metaUpdate.completionTokens;
      }
      if (metaUpdate.cacheHitTokens !== undefined) {
        mapped.sessionCacheHitTokens = (slice.sessionCacheHitTokens ?? 0) + metaUpdate.cacheHitTokens;
      }
      if (metaUpdate.cacheMissTokens !== undefined) {
        mapped.sessionCacheMissTokens =
          (slice.sessionCacheMissTokens ?? 0) + metaUpdate.cacheMissTokens;
      }

      updated.set(sessionId, { ...slice, ...mapped });
      set({ byId: updated });
    },

    onCompactLifecycle: (life) => {
      const sessionId = life.session_id;
      if (life.stage === "started") {
        patch(sessionId, {
          compacting: true,
          ...(life.trigger === "auto" ? { turnPhase: "compacting" as const } : {}),
        });
        return;
      }
      if (life.stage === "succeeded") {
        patch(sessionId, { compacting: false });
        if (life.trigger === "auto") {
          useNotificationStore.getState().add(sessionId, "Context compacted");
        }
        return;
      }
      patch(sessionId, { compacting: false });
      if (life.trigger === "auto") {
        useToastStore.getState().showToast(
          life.error?.message ?? "Compact failed",
          "error",
          8000,
        );
      }
    },

    onLifecycleTurnFinished: (sessionId, finishedTurnId) => {
      const current = getSlice(get().byId, sessionId);
      if (!shouldApplyTurnEnd(current.currentTurnId, current.runState, finishedTurnId)) {
        debugTrace("turn", "lifecycle.finished.skipped", {
          sessionId,
          finishedTurnId: finishedTurnId ?? null,
          currentTurnId: current.currentTurnId,
          runState: current.runState,
        });
        return;
      }
      debugTrace("turn", "lifecycle.finished", {
        sessionId,
        finishedTurnId: finishedTurnId ?? null,
        currentTurnId: current.currentTurnId,
        runState: current.runState,
      });
      flushStreamSession(sessionId);
      useMessageStore.getState().finalizeTurn(sessionId, current.currentTurnId ?? "");
      get().applySnapshotTurn(sessionId, null);
    },

    onTurnFinished: (tf) => {
      const sessionId = tf.session_id || tf.snapshot.session_id;
      const current = getSlice(get().byId, sessionId);
      const applies = shouldApplyTurnEnd(
        current.currentTurnId,
        current.runState,
        tf.turn_id,
      );
      debugTrace("turn", applies ? "finished" : "finished.skipped", {
        sessionId,
        finished: tf.turn_id,
        currentTurnId: current.currentTurnId,
        runState: current.runState,
        reason: tf.reason,
        error: tf.error?.message ?? null,
        committedEnd: tf.snapshot.buffer?.next_seq,
        bufferLen: tf.snapshot.buffer?.next_seq,
      });

      const snap = tf.snapshot;
      const notice = turnEndNoticeFrom(tf);
      if (!applies) {
        get().applySnapshotMeter(sessionId, snap);
        if (current.runState === "idle" && notice) {
          useMessageStore.getState().setTurnEndNotice(sessionId, notice);
        }
        return;
      }

      const tts = snap.last_turn_token_stats ?? tf.turn_token_stats;
      const cum = snap.cumulative_token_stats;
      const turnId = tf.turn_id || current.currentTurnId;

      flushStreamSession(sessionId);
      useMessageStore.getState().finalizeTurn(sessionId, turnId ?? "");
      if (notice) {
        useMessageStore.getState().setTurnEndNotice(sessionId, notice);
      }

      const localEnd =
        useMessageStore.getState().bySession.get(sessionId)?.toSeq ?? 0;
      const serverEnd = snap.buffer?.next_seq ?? 0;
      if (serverEnd > localEnd) {
        void useMessageStore.getState().loadRange(sessionId, localEnd, serverEnd);
      }

      patch(sessionId, {
        currentTurnId: null,
        turnPhase: null,
        turnStep: null,
        turnStepMax: null,
        pendingCancel: false,
        runState: "idle",
        pendingPermission: null,
        contextWindow: snap.context_window ?? 0,
        contextTokensEstimate: snap.context_tokens_estimate ?? 0,
        compactEligible: snap.compact_eligible ?? false,
        compacting: snap.compacting ?? false,
        ...(tts
          ? {
              lastTurnPromptTokens: tts.prompt_tokens ?? 0,
              lastTurnCompletionTokens: tts.completion_tokens ?? 0,
              lastTurnCacheHitTokens: tts.cache_hit_tokens ?? 0,
              lastTurnCacheMissTokens: tts.cache_miss_tokens ?? 0,
            }
          : {}),
        ...(cum
          ? {
              sessionPromptTokens: cum.prompt_tokens ?? 0,
              sessionCompletionTokens: cum.completion_tokens ?? 0,
              sessionCacheHitTokens: cum.cache_hit_tokens ?? 0,
              sessionCacheMissTokens: cum.cache_miss_tokens ?? 0,
            }
          : {}),
      });
    },

    applySnapshotTurn: (sessionId, turn) => {
      if (!turn) {
        patch(sessionId, {
          turnPhase: null,
          turnStep: null,
          turnStepMax: null,
          runState: "idle",
          currentTurnId: null,
        });
        return;
      }

      const existing = getSlice(get().byId, sessionId);
      const runState = existing.pendingCancel
        ? "cancelling"
        : deriveRunState(turn);

      patch(sessionId, {
        turnPhase: turn.phase,
        turnStep: turn.step,
        turnStepMax: turn.step_max,
        runState,
        currentTurnId: turn.turn_id,
      });
    },

    applySnapshotMeter: (sessionId, snap) => {
      const tts = snap.last_turn_token_stats;
      const cum = snap.cumulative_token_stats;
      // Snapshot meter is authoritative: absent last_turn_token_stats (e.g.
      // post-compact) must clear stale provider occupancy so the ring can
      // fall back to context_tokens_estimate.
      patch(sessionId, {
        contextWindow: snap.context_window ?? 0,
        contextTokensEstimate: snap.context_tokens_estimate ?? 0,
        compactEligible: snap.compact_eligible ?? false,
        compacting: snap.compacting ?? false,
        lastTurnPromptTokens: tts?.prompt_tokens ?? 0,
        lastTurnCompletionTokens: tts?.completion_tokens ?? 0,
        lastTurnCacheHitTokens: tts?.cache_hit_tokens ?? 0,
        lastTurnCacheMissTokens: tts?.cache_miss_tokens ?? 0,
        ...(cum
          ? {
              sessionPromptTokens: cum.prompt_tokens ?? 0,
              sessionCompletionTokens: cum.completion_tokens ?? 0,
              sessionCacheHitTokens: cum.cache_hit_tokens ?? 0,
              sessionCacheMissTokens: cum.cache_miss_tokens ?? 0,
            }
          : {}),
        ...(snap.todos !== undefined
          ? todoPatchRetaining(getSlice(get().byId, sessionId), snap.todos)
          : {}),
      });
    },

    resetTurn: (sessionId) => {
      clearStreamSession(sessionId);
      patch(sessionId, emptySlice());
    },

    flushPendingStream: (sessionId) => {
      flushStreamSession(sessionId);
    },

    clearPendingStream: (sessionId) => {
      clearStreamSession(sessionId);
    },
  };
});

attachSiblingStores({ turn: useTurnStore });