import { create } from "zustand";
import {
  applyStreamEvent,
  hydrateUserDetailBefore,
  isHumanUserRow,
  isWellFormedBufferRow,
  itemFromRow,
  isStreamFailureEvent,
  itemAuthorityId,
  itemPlainText,
  markFunctionCallsFailed,
  sealMismatchError,
} from "../api/adapter";
import type {
  BufferItemNotification,
  BufferLoaded,
  HumanRow,
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
  bySeq: Map<number, HumanRow>;
  messages: HumanRow[];
  /** Optimistic composer row; not a seq key. At most one. */
  pendingUser: PendingUser | null;
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
  /** item_id / call_id → seq for stream deltas. Points at the latest seq. */
  itemIdToSeq: Map<string, number>;
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
        kind: "item/user",
        body: slice.pendingUser.item,
      },
    ],
  };
}

export function displayMessages(slice: MessageSlice | undefined): HumanRow[] {
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

function sortedMessages(bySeq: Map<number, HumanRow>): HumanRow[] {
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

function rememberRowItem(itemIdToSeq: Map<string, number>, row: HumanRow): void {
  const item = itemFromRow(row);
  const aid = item && itemAuthorityId(item);
  if (aid) itemIdToSeq.set(aid, row.seq);
}

function streamEventItemIdHint(event: ResponseStreamEvent): string | undefined {
  if (typeof (event as { item_id?: unknown }).item_id === "string") {
    const id = (event as { item_id: string }).item_id;
    if (id.length > 0) return id;
  }
  const nested = (event as { item?: { id?: unknown; call_id?: unknown } }).item;
  if (nested && typeof nested === "object") {
    if (typeof nested.id === "string" && nested.id.length > 0) return nested.id;
    if (typeof nested.call_id === "string" && nested.call_id.length > 0) return nested.call_id;
  }
  return undefined;
}

function isSealedItem(item: Item): boolean {
  if ("status" in item && typeof item.status === "string") {
    return item.status === "completed" || item.status === "failed" || item.status === "incomplete";
  }
  return true;
}

function rowFromEvent(ev: WireBufferEvent, streaming: boolean): HumanRow {
  return { ...ev, streaming };
}

function malformedSeq(ev: unknown): number | undefined {
  if (ev === null || typeof ev !== "object") return undefined;
  const seq = (ev as { seq?: unknown }).seq;
  return typeof seq === "number" && Number.isFinite(seq) && seq >= 0 ? seq : undefined;
}

function upsertEvents(slice: MessageSlice, events: WireBufferEvent[]): MessageSlice {
  const bySeq = new Map(slice.bySeq);
  const itemIdToSeq = new Map(slice.itemIdToSeq);
  let pendingUser = slice.pendingUser;
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
    const nextRow = rowFromEvent(ev, false);
    const nextItem = itemFromRow(nextRow);
    nextRow.streaming = nextItem ? !isSealedItem(nextItem) : false;
    const prev = bySeq.get(ev.seq);
    const prevItem = prev && itemFromRow(prev);
    if (prev && prevItem && nextItem) {
      const mismatch = sealMismatchError(prevItem, nextItem);
      if (mismatch) {
        shapeError = mismatch;
        useToastStore.getState().showToast(mismatch, "error");
      }
    }
    bySeq.set(ev.seq, { ...nextRow, streaming: nextRow.streaming });
    rememberRowItem(itemIdToSeq, nextRow);
    if (pendingUser && nextItem && isHumanUserRow(nextRow) && itemPlainText(pendingUser.item) === itemPlainText(nextItem)) {
      pendingUser = null;
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
    fromSeq = empty ? Math.min(...seqs) : Math.min(slice.fromSeq, Math.min(...seqs));
    toSeq = Math.max(slice.toSeq, Math.max(...seqs) + 1);
  }
  return {
    ...slice, bySeq, messages, pendingUser, itemIdToSeq, fromSeq, toSeq,
    userDetailBefore: hydrateUserDetailBefore(fromSeq, undefined, slice.userDetailBefore), shapeError,
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
      next.userDetailBefore = hydrateUserDetailBefore(
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
        const callId = bufferItem.type === "function_call" && "call_id" in bufferItem && typeof bufferItem.call_id === "string"
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

    applyStreamEvent: (sessionId, turnId, _step, event) => {
      const slice = getSlice(get().bySession, sessionId);
      const turn = useTurnStore.getState().byId.get(sessionId);

      if (isStreamFailureEvent(event)) {
        const bySeq = new Map(slice.bySeq);
        for (const [seq, row] of bySeq) {
          const item = itemFromRow(row);
          if (!item) continue;
          const failed = markFunctionCallsFailed([item])[0];
          if (failed !== item) bySeq.set(seq, { ...row, body: failed, streaming: false } as HumanRow);
        }
        patch(sessionId, { bySeq, messages: sortedMessages(bySeq) });
        return;
      }

      const itemIdHint = streamEventItemIdHint(event);
      if (!itemIdHint) return;
      const seq = slice.itemIdToSeq.get(itemIdHint);
      if (seq == null) return;
      const existing = slice.bySeq.get(seq);
      const existingItem = existing && itemFromRow(existing);
      if (!existing || !existingItem || isSealedItem(existingItem)) return;
      if (turn && (turn.runState !== "running" || turn.currentTurnId !== turnId)) return;

      const result = applyStreamEvent(existingItem, event);
      if (result.kind === "noop") return;
      if (result.kind === "error") {
        reportShapeError(patch, sessionId, result.message);
        return;
      }
      const bySeq = new Map(slice.bySeq);
      bySeq.set(seq, { ...existing, body: result.item, streaming: true } as HumanRow);
      patch(sessionId, { bySeq, messages: sortedMessages(bySeq), shapeError: null });
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
      const bySeq = new Map<number, HumanRow>();
      const itemIdToSeq = new Map<string, number>();
      for (const [seq, row] of slice.bySeq) {
        if (seq < rev.next_seq) {
          bySeq.set(seq, row);
          rememberRowItem(itemIdToSeq, row);
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
