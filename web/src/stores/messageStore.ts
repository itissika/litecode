import { create } from "zustand";
import {
  hydrateUserDetailBefore,
  isWellFormedBufferRow,
  itemFromRow,
  itemPlainText,
  optimisticUserSealText,
  sealMismatchError,
} from "../api/adapter";
import type {
  BufferItemNotification,
  BufferLoaded,
  HumanRow,
  Item,
  SubagentBound,
  WireBufferEvent,
} from "../api/types";
import { debugTrace } from "../lib/debugTrace";
import { useConnectionStore, attachSiblingStores } from "./connectionStore";
import { useToastStore } from "./toastStore";

const HISTORY_PAGE = 40;

export interface PendingUser {
  clientId: string;
  item: Item;
}

/**
 * The queued batch as an in-flight bubble in the transcript area.
 *
 * Created from the queue mirror, kept after the server claims it (the queue
 * empties before the durable row lands) and sealed by the matching `item/user`
 * row — so the bubble becomes the real message instead of blinking out. It is
 * never a log row: no seq, no revert anchor, and it disappears on its own if
 * the batch never becomes durable (snapshot re-sync, recall).
 */
export interface PendingQueueBubble {
  texts: string[];
  /** Exactly what the server merges into one user row (`join("\n\n")`). */
  joined: string;
}

export interface MessageSlice {
  /** Seq → row. Sorted projection is `messages`. */
  bySeq: Map<number, HumanRow>;
  messages: HumanRow[];
  /** Optimistic composer row; not a seq key. At most one. */
  pendingUser: PendingUser | null;
  /** In-flight bubble for the queued batch. At most one. */
  pendingQueue: PendingQueueBubble | null;
  /**
   * Seq of the durable row that took over from `pendingQueue`, set in the same
   * update that drops the bubble: the settle animation needs the row's identity
   * (a seq, not the batch text) so it can fire wherever the row landed and be
   * cleared once it has played.
   */
  landedQueueSeq: number | null;
  /**
   * `messages` plus pending user row. Stable until the next slice patch —
   * zustand selectors must not allocate this on each snapshot.
   */
  display: HumanRow[];
  /** Loaded window `[fromSeq, toSeq)`. */
  fromSeq: number;
  toSeq: number;
  /**
   * Server count of user-detail rows with seq `< fromSeq` (`buffer/load`
   * `user_detail_before`). 0 when the loaded window starts at seq 0.
   */
  userDetailBefore: number;
  loadingHistory: boolean;
  /** True after the first buffer/load for this session (including empty). */
  hydrated: boolean;
  shapeError: string | null;
  subagentBindings: Record<string, string>;
  blockLogGrowth: boolean;
  turnEndNotice: TurnEndNotice | null;
}

export interface TurnEndNotice {
  kind: "error";
  message: string;
}

export const EMPTY_DISPLAY: HumanRow[] = [];

export const EMPTY_SLICE: MessageSlice = {
  bySeq: new Map(),
  messages: EMPTY_DISPLAY,
  pendingUser: null,
  pendingQueue: null,
  landedQueueSeq: null,
  display: EMPTY_DISPLAY,
  fromSeq: 0,
  toSeq: 0,
  userDetailBefore: 0,
  loadingHistory: false,
  hydrated: false,
  shapeError: null,
  subagentBindings: {},
  blockLogGrowth: false,
  turnEndNotice: null,
};

export function emptySlice(): MessageSlice {
  return {
    ...EMPTY_SLICE,
    bySeq: new Map(),
    messages: EMPTY_DISPLAY,
    display: EMPTY_DISPLAY,
    subagentBindings: {},
  };
}

function withDisplay(slice: MessageSlice): MessageSlice {
  if (!slice.pendingUser) {
    return slice.display === slice.messages
      ? slice
      : { ...slice, display: slice.messages };
  }
  return {
    ...slice,
    display: [
      ...slice.messages,
      {
        // The composer bubble is not a log row, so it carries no lifecycle of its
        // own: the user's own text is never "in progress".
        seq: -1,
        kind: "item/user",
        state: "final",
        body: slice.pendingUser.item,
      },
    ],
  };
}

export function displayMessages(slice: MessageSlice | undefined): HumanRow[] {
  if (!slice) return EMPTY_DISPLAY;
  return slice.display;
}

function getSlice(
  byId: Map<string, MessageSlice>,
  sessionId: string,
): MessageSlice {
  let slice = byId.get(sessionId);
  if (!slice) {
    slice = emptySlice();
    byId.set(sessionId, slice);
  }
  return slice;
}

function sortedMessages(bySeq: Map<number, HumanRow>): HumanRow[] {
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

function malformedSeq(ev: unknown): number | undefined {
  if (ev === null || typeof ev !== "object") return undefined;
  const seq = (ev as { seq?: unknown }).seq;
  return typeof seq === "number" && Number.isFinite(seq) && seq >= 0
    ? seq
    : undefined;
}

function upsertEvents(
  slice: MessageSlice,
  events: WireBufferEvent[],
): MessageSlice {
  const bySeq = new Map(slice.bySeq);
  let pendingUser = slice.pendingUser;
  let pendingQueue = slice.pendingQueue;
  let landedQueueSeq = slice.landedQueueSeq;
  let shapeError = slice.shapeError;
  const empty = slice.bySeq.size === 0;

  for (const ev of events) {
    if (!isWellFormedBufferRow(ev)) {
      const seq = malformedSeq(ev);
      const prev = seq != null ? bySeq.get(seq) : undefined;
      if (prev && isWellFormedBufferRow(prev)) {
        shapeError = "buffer/item rejected: missing kind/body";
        useToastStore.getState().showToast(shapeError, "error");
      }
      continue;
    }
    if (slice.blockLogGrowth && ev.seq >= slice.toSeq) continue;
    const nextRow: HumanRow = { ...ev };
    const nextItem = itemFromRow(nextRow);
    const prev = bySeq.get(ev.seq);
    const prevItem = prev && itemFromRow(prev);
    if (prev && prevItem && nextItem) {
      const mismatch = sealMismatchError(prevItem, nextItem);
      if (mismatch) {
        shapeError = mismatch;
        useToastStore.getState().showToast(mismatch, "error");
      }
    }
    bySeq.set(ev.seq, nextRow);
    // `item/user` seals the composer bubble; `plan/execute` lands in its place
    // (same user Item text) and must seal it too, or the row double-renders.
    if (pendingUser) {
      const sealText = optimisticUserSealText(nextRow);
      if (sealText !== null && sealText === itemPlainText(pendingUser.item)) {
        pendingUser = null;
      }
    }
    // The queued batch seals the same way the composer row does: the durable
    // row is the message, so the in-flight bubble hands over to it.
    if (pendingQueue) {
      const sealText = optimisticUserSealText(nextRow);
      if (sealText !== null && sealText === pendingQueue.joined) {
        pendingQueue = null;
        landedQueueSeq = nextRow.seq;
      }
    }
  }

  const messages = sortedMessages(bySeq);
  let fromSeq = slice.fromSeq;
  let toSeq = slice.toSeq;
  const seqs = events
    .filter(isWellFormedBufferRow)
    .map((e) => e.seq)
    .filter((s) => Number.isFinite(s) && s >= 0);
  if (seqs.length > 0) {
    fromSeq = empty
      ? Math.min(...seqs)
      : Math.min(slice.fromSeq, Math.min(...seqs));
    toSeq = Math.max(slice.toSeq, Math.max(...seqs) + 1);
  }
  return {
    ...slice,
    bySeq,
    messages,
    pendingUser,
    pendingQueue,
    landedQueueSeq,
    fromSeq,
    toSeq,
    userDetailBefore: hydrateUserDetailBefore(
      fromSeq,
      undefined,
      slice.userDetailBefore,
    ),
    shapeError,
  };
}

interface MessageState {
  bySession: Map<string, MessageSlice>;
}

interface MessageStore extends MessageState {
  onBufferLoaded: (sessionId: string, loaded: BufferLoaded) => void;
  onBufferItem: (sessionId: string, bi: BufferItemNotification) => void;
  onBufferReverted: (
    sessionId: string,
    rev: { session_id: string; last_seq: number; next_seq: number },
  ) => void;
  onSubagentBound: (sessionId: string, bound: SubagentBound) => void;
  allowLogGrowth: (sessionId: string) => void;

  setTurnEndNotice: (sessionId: string, notice: TurnEndNotice | null) => void;

  pushPendingUser: (sessionId: string, pending: PendingUser) => void;
  discardOptimisticUserMessage: (sessionId: string, clientId: string) => void;
  /**
   * Mirror the queued batch as the in-flight bubble. `null`/empty clears it;
   * non-empty (re)sets it. Callers own the semantics: a live empty list means
   * "claimed, row en route" and must NOT clear, while a snapshot or a recall
   * must.
   */
  setPendingQueue: (sessionId: string, texts: string[] | null) => void;
  /** Called when the settle animation has played (and only then). */
  clearLandedQueueSeq: (sessionId: string) => void;
  loadRange: (
    sessionId: string,
    fromSeq: number,
    toSeq: number,
  ) => Promise<void>;
  loadMoreHistory: (sessionId: string) => void;
  ensureSeqLoaded: (
    sessionId: string,
    seq: number,
    isCurrent?: () => boolean,
  ) => Promise<boolean>;
  revertToUserAnchor: (sessionId: string, k: number) => void;
  revertFiles: (sessionId: string, k: number) => void;
  reset: (sessionId: string) => void;
}

function reportShapeError(
  patch: (sessionId: string, update: Partial<MessageSlice>) => void,
  sessionId: string,
  message: string,
): void {
  patch(sessionId, { shapeError: message });
  useToastStore.getState().showToast(message, "error");
}

export const useMessageStore = create<MessageStore>((set, get) => {
  function patch(sessionId: string, update: Partial<MessageSlice>): void {
    const state = get();
    const bySession = new Map(state.bySession);
    const slice = withDisplay({ ...getSlice(bySession, sessionId), ...update });
    bySession.set(sessionId, slice);
    set({ bySession });
  }

  return {
    bySession: new Map(),

    onBufferLoaded: (sessionId, loaded) => {
      if (!Array.isArray(loaded.events)) {
        reportShapeError(
          patch,
          sessionId,
          "buffer/load rejected: missing events",
        );
        patch(sessionId, { loadingHistory: false });
        return;
      }
      const missingSeq = loaded.events.some((e) => !Number.isFinite(e.seq));
      if (missingSeq) {
        reportShapeError(
          patch,
          sessionId,
          "buffer/load rejected: event missing seq",
        );
        patch(sessionId, { loadingHistory: false });
        return;
      }
      const state = get();
      const slice = getSlice(state.bySession, sessionId);
      const next = upsertEvents(slice, loaded.events);
      // A tail append (the P6 gap catch-up) starts at/after the current window
      // end. Keep the existing window start and user-detail count: the older
      // rows are still held, and `userDetailBefore` counts from the window start,
      // not from the appended tail.
      const tailAppend = slice.toSeq > 0 && loaded.from_seq >= slice.toSeq;
      next.fromSeq = tailAppend ? slice.fromSeq : loaded.from_seq;
      next.toSeq = Math.max(next.toSeq, loaded.to_seq);
      next.userDetailBefore = tailAppend
        ? slice.userDetailBefore
        : hydrateUserDetailBefore(
            loaded.from_seq,
            loaded.user_detail_before,
            slice.userDetailBefore,
          );
      next.loadingHistory = false;
      next.hydrated = true;
      next.subagentBindings = {
        ...slice.subagentBindings,
        ...(loaded.subagent_bindings ?? {}),
      };
      debugTrace("buffer", "load", {
        sessionId,
        fromSeq: loaded.from_seq,
        toSeq: loaded.to_seq,
        events: loaded.events.length,
        rows: next.messages.length,
      });
      const bySession = new Map(state.bySession);
      bySession.set(sessionId, withDisplay(next));
      set({ bySession });
    },

    onSubagentBound: (sessionId, bound) => {
      const slice = getSlice(get().bySession, sessionId);
      patch(sessionId, {
        subagentBindings: {
          ...slice.subagentBindings,
          [bound.call_id]: bound.child_session_id,
        },
      });
    },

    allowLogGrowth: (sessionId) => {
      patch(sessionId, { blockLogGrowth: false, turnEndNotice: null });
    },

    onBufferItem: (sessionId, bi) => {
      if (!isWellFormedBufferRow(bi)) {
        const state = get();
        const slice = getSlice(state.bySession, sessionId);
        const next = upsertEvents(slice, [bi as WireBufferEvent]);
        const bySession = new Map(state.bySession);
        bySession.set(sessionId, withDisplay(next));
        set({ bySession });
        return;
      }
      if (bi.kind === "item/tool_call" && bi.child_session_id) {
        const bufferItem = bi.body;
        const callId =
          bufferItem.type === "function_call" &&
          "call_id" in bufferItem &&
          typeof bufferItem.call_id === "string"
            ? bufferItem.call_id
            : undefined;
        if (callId) {
          const slice = getSlice(get().bySession, sessionId);
          patch(sessionId, {
            subagentBindings: {
              ...slice.subagentBindings,
              [callId]: bi.child_session_id,
            },
          });
        }
      }
      const state = get();
      const slice = getSlice(state.bySession, sessionId);
      const next = upsertEvents(slice, [bi]);
      debugTrace("buffer", "item.sealed", {
        sessionId,
        seq: bi.seq,
        kind: bi.kind,
        rows: next.messages.length,
      });
      const bySession = new Map(state.bySession);
      bySession.set(sessionId, withDisplay(next));
      set({ bySession });
    },

    setTurnEndNotice: (sessionId, notice) => {
      patch(sessionId, { turnEndNotice: notice });
      if (notice) {
        useToastStore.getState().showToast(notice.message, "error", 8000);
      }
    },

    pushPendingUser: (sessionId, pending) => {
      const slice = getSlice(get().bySession, sessionId);
      patch(sessionId, { pendingUser: slice.pendingUser ?? pending });
    },

    discardOptimisticUserMessage: (sessionId, clientId) => {
      const slice = getSlice(get().bySession, sessionId);
      if (slice.pendingUser?.clientId !== clientId) return;
      patch(sessionId, { pendingUser: null });
    },

    setPendingQueue: (sessionId, texts) => {
      const slice = getSlice(get().bySession, sessionId);
      if (texts === null || texts.length === 0) {
        if (slice.pendingQueue === null) return;
        patch(sessionId, { pendingQueue: null });
        return;
      }
      const joined = texts.join("\n\n");
      if (slice.pendingQueue?.joined === joined) return;
      // A new batch is a new arrival: whatever settled before is stale.
      patch(sessionId, {
        pendingQueue: { texts: [...texts], joined },
        landedQueueSeq: null,
      });
    },

    clearLandedQueueSeq: (sessionId) => {
      const slice = getSlice(get().bySession, sessionId);
      if (slice.landedQueueSeq === null) return;
      patch(sessionId, { landedQueueSeq: null });
    },

    onBufferReverted: (sessionId, rev) => {
      const slice = getSlice(get().bySession, sessionId);
      const bySeq = new Map<number, HumanRow>();
      // `last_seq` is the surviving tail; `next_seq` is the allocator high-water
      // and does not move back, so after a revert it can sit well above the
      // tail. Keeping `seq < next_seq` would leave the deleted rows in the UI.
      for (const [seq, row] of slice.bySeq) {
        if (seq <= rev.last_seq) {
          bySeq.set(seq, row);
        }
      }
      const messages = sortedMessages(bySeq);
      debugTrace("buffer", "reverted", {
        sessionId,
        nextSeq: rev.next_seq,
        dropped: slice.messages.length - messages.length,
      });
      patch(sessionId, {
        bySeq,
        messages,
        pendingUser: null,
        // A revert voids the hand-over: the bubble can no longer seal (its row
        // is gone) and the settle it was waiting for is meaningless.
        pendingQueue: null,
        landedQueueSeq: null,
        fromSeq: Math.min(slice.fromSeq, rev.next_seq),
        toSeq: rev.next_seq,
        userDetailBefore: hydrateUserDetailBefore(
          Math.min(slice.fromSeq, rev.next_seq),
          undefined,
          slice.userDetailBefore,
        ),
        shapeError: null,
        blockLogGrowth: true,
        turnEndNotice: null,
      });
    },

    loadRange: async (sessionId, fromSeq, toSeq) => {
      const loaded = await useConnectionStore
        .getState()
        .sendRpc<BufferLoaded>("buffer/load", {
          from_seq: fromSeq,
          to_seq: toSeq,
          session_id: sessionId,
        });
      get().onBufferLoaded(sessionId, loaded);
    },

    loadMoreHistory: (sessionId) => {
      const slice = getSlice(get().bySession, sessionId);
      if (slice.fromSeq <= 0) return;
      const toSeq = slice.fromSeq;
      const fromSeq = Math.max(0, toSeq - HISTORY_PAGE);
      if (fromSeq >= toSeq) return;
      patch(sessionId, { loadingHistory: true });
      useConnectionStore
        .getState()
        .sendRpc<BufferLoaded>("buffer/load", {
          from_seq: fromSeq,
          to_seq: toSeq,
          session_id: sessionId,
        })
        .then((loaded) => {
          get().onBufferLoaded(sessionId, loaded);
        })
        .catch(() => {
          patch(sessionId, { loadingHistory: false });
        });
    },

    ensureSeqLoaded: async (sessionId, seq, isCurrent = () => true) => {
      while (isCurrent()) {
        const slice = get().bySession.get(sessionId) ?? emptySlice();
        if (slice.bySeq.has(seq)) return true;
        if (!slice.hydrated) return false;
        if (seq >= slice.fromSeq && seq < slice.toSeq) return false;
        if (slice.fromSeq <= 0) return false;
        if (seq >= slice.toSeq) return false;
        const toSeq = slice.fromSeq;
        const fromSeq = Math.max(0, toSeq - HISTORY_PAGE);
        if (fromSeq >= toSeq) return false;
        await get().loadRange(sessionId, fromSeq, toSeq);
      }
      return false;
    },

    revertToUserAnchor: (sessionId, k) => {
      useConnectionStore
        .getState()
        .sendRpc("session/revert-to-user-anchor", { k, session_id: sessionId })
        .catch((err) => {
          useToastStore
            .getState()
            .showToast(
              err instanceof Error ? err.message : "Revert failed",
              "error",
            );
        });
    },

    revertFiles: (sessionId, k) => {
      useConnectionStore
        .getState()
        .sendRpc("session/revert-files", { k, session_id: sessionId })
        .catch((err) => {
          useToastStore
            .getState()
            .showToast(
              err instanceof Error ? err.message : "Revert failed",
              "error",
            );
        });
    },

    reset: (sessionId) => {
      const bySession = new Map(get().bySession);
      bySession.set(sessionId, emptySlice());
      set({ bySession });
    },
  };
});

attachSiblingStores({ message: useMessageStore });
