import { create } from "zustand";
import type { ChatRow } from "../api/adapter";
import {
  applyStreamEvent,
  isHumanUserRow,
  isStreamFailureEvent,
  itemAuthorityId,
  itemPlainText,
  markFunctionCallsFailed,
  sealMismatchError,
} from "../api/adapter";
import type {
  BufferItemNotification,
  BufferLoaded,
  Item,
  ResponseStreamEvent,
  SubagentBound,
  WireBufferEvent,
} from "../api/types";
import { debugTrace } from "../lib/debugTrace";
import { useConnectionStore, attachSiblingStores } from "./connectionStore";
import { useToastStore } from "./toastStore";
import { useTurnStore } from "./turnStore";

const HISTORY_PAGE = 40;

export interface PendingUser {
  clientId: string;
  item: Item;
}

export interface MessageSlice {
  /** Seq → row. Sorted projection is `messages`. */
  bySeq: Map<number, ChatRow>;
  messages: ChatRow[];
  /** Optimistic composer row; not a seq key. At most one. */
  pendingUser: PendingUser | null;
  /**
   * `messages` plus pending user row. Stable until the next slice patch —
   * zustand selectors must not allocate this on each snapshot.
   */
  display: ChatRow[];
  /** Loaded window `[fromSeq, toSeq)`. */
  fromSeq: number;
  toSeq: number;
  /**
   * Append-origin users with seq `< fromSeq` when the window starts at 0;
   * otherwise 0 until history is loaded from seq 0.
   */
  userDetailBefore: number;
  loadingHistory: boolean;
  shapeError: string | null;
  subagentBindings: Record<string, string>;
  blockLogGrowth: boolean;
  turnEndNotice: TurnEndNotice | null;
  /** item_id / call_id → seq for stream deltas. Points at the latest seq. */
  itemIdToSeq: Map<string, number>;
}

export interface TurnEndNotice {
  kind: "error";
  message: string;
}

export const EMPTY_DISPLAY: ChatRow[] = [];

export const EMPTY_SLICE: MessageSlice = {
  bySeq: new Map(),
  messages: EMPTY_DISPLAY,
  pendingUser: null,
  display: EMPTY_DISPLAY,
  fromSeq: 0,
  toSeq: 0,
  userDetailBefore: 0,
  loadingHistory: false,
  shapeError: null,
  subagentBindings: {},
  blockLogGrowth: false,
  turnEndNotice: null,
  itemIdToSeq: new Map(),
};

export function emptySlice(): MessageSlice {
  return {
    ...EMPTY_SLICE,
    bySeq: new Map(),
    messages: EMPTY_DISPLAY,
    display: EMPTY_DISPLAY,
    subagentBindings: {},
    itemIdToSeq: new Map(),
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
        seq: -1,
        item: slice.pendingUser.item,
        eventType: "item/user",
        surfaceOp: "append",
      },
    ],
  };
}

export function displayMessages(slice: MessageSlice | undefined): ChatRow[] {
  if (!slice) return EMPTY_DISPLAY;
  return slice.display;
}

function getSlice(byId: Map<string, MessageSlice>, sessionId: string): MessageSlice {
  let slice = byId.get(sessionId);
  if (!slice) {
    slice = emptySlice();
    byId.set(sessionId, slice);
  }
  return slice;
}

function sortedMessages(bySeq: Map<number, ChatRow>): ChatRow[] {
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

function rememberItemSeq(itemIdToSeq: Map<string, number>, item: Item, seq: number): void {
  const aid = itemAuthorityId(item);
  if (aid) itemIdToSeq.set(aid, seq);
}

function streamEventItemIdHint(event: ResponseStreamEvent): string | undefined {
  if (typeof (event as { item_id?: unknown }).item_id === "string") {
    const id = (event as { item_id: string }).item_id;
    if (id.length > 0) return id;
  }
  const nested = (event as { item?: { id?: unknown; call_id?: unknown } }).item;
  if (nested && typeof nested === "object") {
    if (typeof nested.id === "string" && nested.id.length > 0) return nested.id;
    if (typeof nested.call_id === "string" && nested.call_id.length > 0) {
      return nested.call_id;
    }
  }
  return undefined;
}

function isSealedItem(item: Item): boolean {
  if ("status" in item && typeof item.status === "string") {
    return item.status === "completed" || item.status === "failed" || item.status === "incomplete";
  }
  return true;
}

function rowFromEvent(ev: WireBufferEvent, streaming: boolean): ChatRow {
  return {
    seq: ev.seq,
    item: ev.item,
    eventType: ev.type,
    surfaceOp: ev.surface_op,
    streaming,
    childSessionId: ev.child_session_id,
  };
}

function upsertEvents(slice: MessageSlice, events: WireBufferEvent[]): MessageSlice {
  const bySeq = new Map(slice.bySeq);
  const itemIdToSeq = new Map(slice.itemIdToSeq);
  let pendingUser = slice.pendingUser;
  let shapeError = slice.shapeError;

  const empty = slice.bySeq.size === 0;

  for (const ev of events) {
    if (!Number.isFinite(ev.seq) || ev.seq < 0) continue;
    if (slice.blockLogGrowth && ev.seq >= slice.toSeq) continue;

    const prev = bySeq.get(ev.seq);
    const live = !isSealedItem(ev.item);
    if (!prev) {
      bySeq.set(ev.seq, rowFromEvent(ev, live));
    } else {
      const mismatch = sealMismatchError(prev.item, ev.item);
      if (mismatch) {
        shapeError = mismatch;
        useToastStore.getState().showToast(mismatch, "error");
        bySeq.set(ev.seq, rowFromEvent(ev, live));
      } else {
        bySeq.set(ev.seq, {
          ...prev,
          item: ev.item,
          eventType: ev.type,
          surfaceOp: ev.surface_op ?? prev.surfaceOp,
          streaming: live,
          childSessionId: ev.child_session_id ?? prev.childSessionId,
        });
      }
    }
    rememberItemSeq(itemIdToSeq, ev.item, ev.seq);

    if (pendingUser && isHumanUserRow(rowFromEvent(ev, false))) {
      if (itemPlainText(pendingUser.item) === itemPlainText(ev.item)) {
        pendingUser = null;
      }
    }
  }

  const messages = sortedMessages(bySeq);
  let fromSeq = slice.fromSeq;
  let toSeq = slice.toSeq;
  const seqs = events.map((e) => e.seq).filter((s) => Number.isFinite(s) && s >= 0);
  if (seqs.length > 0) {
    const minSeq = Math.min(...seqs);
    const maxExcl = Math.max(...seqs) + 1;
    fromSeq = empty ? minSeq : Math.min(slice.fromSeq, minSeq);
    toSeq = Math.max(slice.toSeq, maxExcl);
  }

  return {
    ...slice,
    bySeq,
    messages,
    pendingUser,
    itemIdToSeq,
    fromSeq,
    toSeq,
    userDetailBefore: fromSeq === 0 ? 0 : slice.userDetailBefore,
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

  applyStreamEvent: (
    sessionId: string,
    turnId: string,
    step: number,
    event: ResponseStreamEvent,
  ) => void;
  finalizeTurn: (sessionId: string, turnId: string) => void;
  setTurnEndNotice: (sessionId: string, notice: TurnEndNotice | null) => void;

  pushPendingUser: (sessionId: string, pending: PendingUser) => void;
  discardOptimisticUserMessage: (sessionId: string, clientId: string) => void;
  loadRange: (sessionId: string, fromSeq: number, toSeq: number) => Promise<void>;
  loadMoreHistory: (sessionId: string) => void;
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
        reportShapeError(patch, sessionId, "buffer/load rejected: missing events");
        patch(sessionId, { loadingHistory: false });
        return;
      }
      const missingSeq = loaded.events.some((e) => !Number.isFinite(e.seq));
      if (missingSeq) {
        reportShapeError(patch, sessionId, "buffer/load rejected: event missing seq");
        patch(sessionId, { loadingHistory: false });
        return;
      }
      const state = get();
      const slice = getSlice(state.bySession, sessionId);
      const next = upsertEvents(slice, loaded.events);
      next.fromSeq = loaded.from_seq;
      next.toSeq = Math.max(next.toSeq, loaded.to_seq);
      next.userDetailBefore = loaded.from_seq === 0 ? 0 : slice.userDetailBefore;
      next.loadingHistory = false;
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
      if (!Number.isFinite(bi.seq)) return;
      if (bi.child_session_id && bi.item.type === "function_call") {
        const callId = "call_id" in bi.item ? bi.item.call_id : undefined;
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
        type: bi.type,
        rows: next.messages.length,
      });
      const bySession = new Map(state.bySession);
      bySession.set(sessionId, withDisplay(next));
      set({ bySession });
    },

    applyStreamEvent: (sessionId, turnId, _step, event) => {
      const slice = getSlice(get().bySession, sessionId);
      const turn = useTurnStore.getState().byId.get(sessionId);

      if (isStreamFailureEvent(event)) {
        const bySeq = new Map(slice.bySeq);
        for (const [seq, row] of bySeq) {
          const failed = markFunctionCallsFailed([row.item])[0];
          if (failed !== row.item) {
            bySeq.set(seq, { ...row, item: failed, streaming: false });
          }
        }
        patch(sessionId, { bySeq, messages: sortedMessages(bySeq) });
        return;
      }

      const itemIdHint = streamEventItemIdHint(event);
      if (!itemIdHint) return;
      const seq = slice.itemIdToSeq.get(itemIdHint);
      if (seq == null) return;
      const existing = slice.bySeq.get(seq);
      if (!existing) return;
      // Sealed seq: ignore late / reused item_id deltas (G4).
      if (isSealedItem(existing.item)) return;
      if (
        turn &&
        (turn.runState !== "running" || turn.currentTurnId !== turnId)
      ) {
        return;
      }

      const result = applyStreamEvent(existing.item, event);
      if (result.kind === "noop") return;
      if (result.kind === "error") {
        reportShapeError(patch, sessionId, result.message);
        return;
      }

      const bySeq = new Map(slice.bySeq);
      bySeq.set(seq, { ...existing, item: result.item, streaming: true });
      patch(sessionId, {
        bySeq,
        messages: sortedMessages(bySeq),
        shapeError: null,
      });
    },

    finalizeTurn: (sessionId, _turnId) => {
      const slice = getSlice(get().bySession, sessionId);
      const bySeq = new Map(slice.bySeq);
      for (const [seq, row] of bySeq) {
        if (row.streaming) bySeq.set(seq, { ...row, streaming: false });
      }
      patch(sessionId, { bySeq, messages: sortedMessages(bySeq) });
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

    onBufferReverted: (sessionId, rev) => {
      const slice = getSlice(get().bySession, sessionId);
      const bySeq = new Map<number, ChatRow>();
      const itemIdToSeq = new Map<string, number>();
      for (const [seq, row] of slice.bySeq) {
        if (seq < rev.next_seq) {
          bySeq.set(seq, row);
          rememberItemSeq(itemIdToSeq, row.item, seq);
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
        itemIdToSeq,
        pendingUser: null,
        fromSeq: Math.min(slice.fromSeq, rev.next_seq),
        toSeq: rev.next_seq,
        userDetailBefore: rev.next_seq === 0 ? 0 : slice.userDetailBefore,
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

    revertToUserAnchor: (sessionId, k) => {
      useConnectionStore
        .getState()
        .sendRpc("session/revert-to-user-anchor", { k, session_id: sessionId })
        .catch((err) => {
          useToastStore.getState().showToast(
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
          useToastStore.getState().showToast(
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
