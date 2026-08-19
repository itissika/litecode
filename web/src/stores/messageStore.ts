import { create } from "zustand";
import type { ChatRow } from "../api/adapter";
import {
  applyStreamEvent,
  committedIdentity,
  isStreamFailureEvent,
  isUserMessage,
  itemAuthorityId,
  itemPlainText,
  liveItemRowId,
  markFunctionCallsFailed,
  rowBufferIndex,
  sealProjectionRow,
  wireRowKind,
} from "../api/adapter";
import type {
  BufferItemNotification,
  BufferLoaded,
  Item,
  ResponseStreamEvent,
  SubagentBound,
} from "../api/types";
import { debugTrace } from "../lib/debugTrace";
import { useConnectionStore, attachSiblingStores } from "./connectionStore";
import { useToastStore } from "./toastStore";
import { useTurnStore } from "./turnStore";

const HISTORY_PAGE = 40;

export interface MessageSlice {
  /** Projection rows — each row is an authority Item (live shell or sealed). */
  messages: ChatRow[];
  bufferViewStart: number;
  bufferViewEnd: number;
  /**
   * Exclusive end of sealed history ordinals in this slice (`max(bufferIndex)+1`),
   * or the server revert point while `blockLogGrowth` is set.
   * Pagination/fill only — never an ingest gap gate.
   */
  committedBufferEnd: number;
  /**
   * Absolute user-detail count with history ordinal `< bufferViewStart`.
   * From the latest `buffer/load` that extended (or set) the view start.
   */
  userDetailBefore: number;
  loadingHistory: boolean;
  /** Explicit error when a stream/buffer event cannot map to Item authority. */
  shapeError: string | null;
  /** `subagent_launch` call_id → child session id (live bind + buffer/load). */
  subagentBindings: Record<string, string>;
  /** After revert, refuse log-growth appends until the next turn starts. */
  blockLogGrowth: boolean;
  /** Durable panel notice for the last turn end (error / max steps). */
  turnEndNotice: TurnEndNotice | null;
}

export interface TurnEndNotice {
  kind: "error";
  message: string;
}

export const EMPTY_SLICE: MessageSlice = {
  messages: [],
  bufferViewStart: 0,
  bufferViewEnd: 0,
  committedBufferEnd: 0,
  userDetailBefore: 0,
  loadingHistory: false,
  shapeError: null,
  subagentBindings: {},
  blockLogGrowth: false,
  turnEndNotice: null,
};

export function emptySlice(): MessageSlice {
  return { ...EMPTY_SLICE, subagentBindings: {} };
}

function getSlice(byId: Map<string, MessageSlice>, sessionId: string): MessageSlice {
  let slice = byId.get(sessionId);
  if (!slice) {
    slice = emptySlice();
    byId.set(sessionId, slice);
  }
  return slice;
}

interface MessageState {
  bySession: Map<string, MessageSlice>;
}

interface BufferRowItem {
  bufferIndex: number;
  item: Item;
  /** DB row `kind` (`detail` | `compact_checkpoint`) — REV-11 wire. */
  kind?: string;
}

function overlayStats(messages: ChatRow[]): {
  n: number;
  live: number;
  streaming: number;
} {
  let live = 0;
  let streaming = 0;
  for (const row of messages) {
    if (rowBufferIndex(row) == null) {
      live += 1;
      if (row.streaming) streaming += 1;
    }
  }
  return { n: messages.length, live, streaming };
}

function findRowByItemId(messages: ChatRow[], itemId: string): number {
  return messages.findIndex((m) => {
    const aid = itemAuthorityId(m.item);
    return aid === itemId || m.id === liveItemRowId(itemId);
  });
}

/** Resolve live-row key from stream events (top-level item_id, or nested item.id/call_id). */
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

/** Seal lookup: authority id AND Item.type must match (call_id is shared by call + output). */
function findRowForSeal(messages: ChatRow[], committed: Item): number {
  const authId = itemAuthorityId(committed);
  if (!authId) return -1;
  return messages.findIndex((m) => {
    if (m.item.type !== committed.type) return false;
    const aid = itemAuthorityId(m.item);
    // Row-id fallback only while the live shell has no authority id yet.
    // Matching `live-call_A` after the item mutated to call_B seals the wrong slot.
    if (aid) return aid === authId;
    return m.id === liveItemRowId(authId);
  });
}

function orderProjection(messages: ChatRow[]): ChatRow[] {
  return [...messages].sort((left, right) => {
    const li = rowBufferIndex(left);
    const ri = rowBufferIndex(right);
    if (li != null && ri != null) return li - ri;
    if (li != null) return -1;
    if (ri != null) return 1;
    const lu = left.id.startsWith("user-") ? 0 : 1;
    const ru = right.id.startsWith("user-") ? 0 : 1;
    return lu - ru;
  });
}

function derivedCommittedEnd(messages: ChatRow[]): number {
  let max = -1;
  for (const row of messages) {
    const index = rowBufferIndex(row);
    if (index != null && index > max) max = index;
  }
  return max + 1;
}

function isDetailUserEntry(item: Item, kind?: string): boolean {
  return wireRowKind(kind) !== "compact_checkpoint" && isUserMessage(item);
}

function isOptimisticUserShell(row: ChatRow): boolean {
  return row.id.startsWith("user-") && isUserMessage(row.item);
}

function takeAt(
  rows: ChatRow[],
  predicate: (row: ChatRow) => boolean,
): ChatRow | undefined {
  const idx = rows.findIndex(predicate);
  if (idx < 0) return undefined;
  return rows.splice(idx, 1)[0];
}

/** Another row holding this wire ordinal must not keep it as identity. */
function vacateIndex(rows: ChatRow[], index: number, keepId: string): void {
  for (let i = rows.length - 1; i >= 0; i--) {
    if (rowBufferIndex(rows[i]) !== index || rows[i].id === keepId) continue;
    if (itemAuthorityId(rows[i].item)) {
      rows[i] = { ...rows[i], bufferIndex: undefined };
    } else {
      rows.splice(i, 1);
    }
  }
}

function ingestBufferItems(
  set: (partial: Partial<MessageState>) => void,
  getState: () => MessageState,
  sessionId: string,
  items: BufferRowItem[],
  loadMeta?: { start: number; userDetailBefore: number; dropIdleLive?: boolean },
): void {
  if (items.length === 0) return;

  const state = getState();
  const slice = getSlice(state.bySession, sessionId);
  const incoming = slice.blockLogGrowth
    ? items.filter((entry) => entry.bufferIndex < slice.committedBufferEnd)
    : items;
  if (incoming.length === 0) return;

  const rows = [...slice.messages];
  const hadSealed = rows.some((row) => rowBufferIndex(row) != null);
  let shapeError = slice.shapeError;

  for (const entry of incoming) {
    const serverIndex = entry.bufferIndex;
    const kind = wireRowKind(entry.kind);
    const id = committedIdentity(entry.item, serverIndex);

    let source: ChatRow | undefined;
    if (isDetailUserEntry(entry.item, entry.kind)) {
      source = takeAt(
        rows,
        (row) =>
          isOptimisticUserShell(row) &&
          itemPlainText(row.item) === itemPlainText(entry.item),
      );
    }
    if (!source && !loadMeta) {
      const liveIdx = findRowForSeal(rows, entry.item);
      if (liveIdx >= 0 && rowBufferIndex(rows[liveIdx]) == null) {
        source = rows.splice(liveIdx, 1)[0];
      }
    }
    if (!source) {
      source = takeAt(rows, (row) => {
        if (row.id !== id) return false;
        const rowAid = itemAuthorityId(row.item);
        const entryAid = itemAuthorityId(entry.item);
        // Stale live key whose Item mutated to a different authority id.
        if (rowAid && entryAid && rowAid !== entryAid) return false;
        return true;
      });
    } else {
      const dup = rows.findIndex((row) => row.id === id);
      if (dup >= 0) rows.splice(dup, 1);
    }

    vacateIndex(rows, serverIndex, id);

    if (source) {
      const { row, mismatch } = sealProjectionRow(
        source,
        entry.item,
        serverIndex,
        kind,
      );
      if (mismatch) {
        shapeError = mismatch;
        useToastStore.getState().showToast(mismatch, "error");
        rows.push({
          id,
          bufferIndex: serverIndex,
          item: entry.item,
          kind,
          streaming: false,
        });
      } else {
        rows.push({ ...row, id });
      }
    } else {
      rows.push({
        id,
        bufferIndex: serverIndex,
        item: entry.item,
        kind,
        streaming: false,
      });
    }
  }

  const dropIdleLive = loadMeta?.dropIdleLive === true;
  const kept = dropIdleLive
    ? rows.filter((row) => rowBufferIndex(row) != null || row.id.startsWith("user-"))
    : rows;

  const ordered = orderProjection(kept);
  const nextCommitted = slice.blockLogGrowth
    ? slice.committedBufferEnd
    : derivedCommittedEnd(ordered);

  const indices = incoming.map((i) => i.bufferIndex);
  const minIdx = Math.min(...indices);
  const maxIdx = Math.max(...indices) + 1;
  const nextStart = hadSealed ? Math.min(slice.bufferViewStart, minIdx) : minIdx;

  let userDetailBefore = slice.userDetailBefore;
  if (loadMeta && (!hadSealed || loadMeta.start < slice.bufferViewStart)) {
    userDetailBefore = loadMeta.userDetailBefore;
  }

  const nextSlice: MessageSlice = {
    ...slice,
    messages: ordered,
    bufferViewStart: nextStart,
    bufferViewEnd: Math.max(slice.bufferViewEnd, maxIdx),
    committedBufferEnd: nextCommitted,
    userDetailBefore,
    shapeError,
  };

  const bySession = new Map(state.bySession);
  bySession.set(sessionId, nextSlice);
  set({ bySession });
}

interface MessageStore extends MessageState {
  onBufferLoaded: (sessionId: string, loaded: BufferLoaded) => void;
  onBufferItem: (sessionId: string, bi: BufferItemNotification) => void;
  onBufferReverted: (
    sessionId: string,
    rev: { session_id: string; committed_end: number },
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

  pushUserMessage: (sessionId: string, row: ChatRow) => void;
  /** Drop a failed optimistic user row by exact client id (MSG-01). */
  discardOptimisticUserMessage: (sessionId: string, rowId: string) => void;
  loadRange: (sessionId: string, start: number, end: number) => Promise<void>;
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
    const slice = { ...getSlice(bySession, sessionId), ...update };
    bySession.set(sessionId, slice);
    set({ bySession });
  }

  return {
    bySession: new Map(),

    onBufferLoaded: (sessionId, loaded) => {
      if (
        !loaded.indices ||
        loaded.indices.length !== loaded.items.length
      ) {
        reportShapeError(
          patch,
          sessionId,
          "buffer/load rejected: missing or mismatched history indices",
        );
        patch(sessionId, { loadingHistory: false });
        return;
      }
      const itemList: BufferRowItem[] = loaded.items.map((item, i) => ({
        bufferIndex: loaded.indices[i]!,
        item,
        kind: loaded.kinds?.[i],
      }));
      const turn = useTurnStore.getState().byId.get(sessionId);
      const turnLive =
        turn?.runState === "running" || turn?.runState === "cancelling";
      ingestBufferItems(set, get, sessionId, itemList, {
        start: loaded.start,
        userDetailBefore: loaded.user_detail_before ?? 0,
        dropIdleLive: !turnLive,
      });
      const bindings = loaded.subagent_bindings ?? {};
      const slice = getSlice(get().bySession, sessionId);
      debugTrace("buffer", "load", {
        sessionId,
        start: loaded.start,
        end: loaded.end,
        items: loaded.items.length,
        ...overlayStats(slice.messages),
        committedEnd: slice.committedBufferEnd,
      });
      patch(sessionId, {
        loadingHistory: false,
        subagentBindings: { ...slice.subagentBindings, ...bindings },
      });
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
      const slice = getSlice(get().bySession, sessionId);
      const itemMeta = {
        sessionId,
        index: bi.buffer_index,
        type: bi.item.type,
        committedEnd: slice.committedBufferEnd,
      };

      if (bi.child_session_id && bi.item.type === "function_call") {
        const callId = "call_id" in bi.item ? bi.item.call_id : undefined;
        if (callId) {
          patch(sessionId, {
            subagentBindings: {
              ...getSlice(get().bySession, sessionId).subagentBindings,
              [callId]: bi.child_session_id,
            },
          });
        }
      }

      ingestBufferItems(set, get, sessionId, [
        {
          bufferIndex: bi.buffer_index,
          item: bi.item,
          kind: bi.kind,
        },
      ]);
      const after = getSlice(get().bySession, sessionId);
      debugTrace("buffer", "item.sealed", {
        ...itemMeta,
        kind: wireRowKind(bi.kind),
        ...overlayStats(after.messages),
      });
    },

    applyStreamEvent: (sessionId, turnId, _step, event) => {
      const slice = getSlice(get().bySession, sessionId);
      const turn = useTurnStore.getState().byId.get(sessionId);

      // A turn-level failure (response.failed / error) invalidates every
      // in-flight function_call — no half-streamed call may stay "in_progress".
      // `response.incomplete` is a seal terminal, not a failure: live rows wait
      // for buffer/item.
      if (isStreamFailureEvent(event)) {
        debugTrace("buffer", "stream.failed", {
          sessionId,
          turnId,
          type: event.type,
          ...overlayStats(slice.messages),
        });
        const messages = slice.messages.map((m) => {
          const failed = markFunctionCallsFailed([m.item])[0];
          return failed !== m.item
            ? { ...m, item: failed, streaming: false }
            : m;
        });
        patch(sessionId, { messages });
        return;
      }

      const itemIdHint = streamEventItemIdHint(event);
      const existingIdx = itemIdHint ? findRowByItemId(slice.messages, itemIdHint) : -1;
      // Authority already sealed this slot (buffer/item). Ignore late deltas —
      // rAF batches can land after seal and would append onto the full text.
      if (existingIdx >= 0 && rowBufferIndex(slice.messages[existingIdx]) != null) {
        return;
      }
      if (
        existingIdx < 0 &&
        turn &&
        (turn.runState !== "running" || turn.currentTurnId !== turnId)
      ) {
        return;
      }
      const existingItem =
        existingIdx >= 0 ? slice.messages[existingIdx].item : undefined;

      const result = applyStreamEvent(existingItem, event);
      if (result.kind === "noop") return;
      if (result.kind === "error") {
        reportShapeError(patch, sessionId, result.message);
        return;
      }

      const messages = [...slice.messages];
      const rowId = liveItemRowId(result.itemId);
      const idx = existingIdx >= 0 ? existingIdx : findRowByItemId(messages, result.itemId);
      if (idx >= 0) {
        messages[idx] = {
          ...messages[idx],
          id: rowId,
          item: result.item,
          streaming: true,
        };
      } else {
        messages.push({
          id: rowId,
          item: result.item,
          streaming: true,
        });
      }
      patch(sessionId, {
        messages: orderProjection(messages),
        shapeError: null,
      });
    },

    finalizeTurn: (sessionId, _turnId) => {
      const slice = getSlice(get().bySession, sessionId);
      let droppedLive = 0;
      const messages = slice.messages.filter((m) => {
        if (m.id.startsWith("live-") && rowBufferIndex(m) == null) {
          droppedLive += 1;
          return false;
        }
        return true;
      }).map((m) => (m.streaming ? { ...m, streaming: false } : m));
      debugTrace("buffer", "finalize", {
        sessionId,
        droppedLive,
        ...overlayStats(messages),
      });
      patch(sessionId, {
        messages: orderProjection(messages),
      });
    },

    setTurnEndNotice: (sessionId, notice) => {
      patch(sessionId, { turnEndNotice: notice });
      // Surface turn-end failures (error / max steps / hook blocked) via toast
      // instead of the old persistent in-panel banner.
      if (notice) {
        useToastStore.getState().showToast(notice.message, "error", 8000);
      }
    },

    pushUserMessage: (sessionId, row) => {
      const slice = getSlice(get().bySession, sessionId);
      patch(sessionId, {
        messages: orderProjection([...slice.messages, row]),
      });
    },

    discardOptimisticUserMessage: (sessionId, rowId) => {
      if (!rowId.startsWith("user-")) return;
      const slice = getSlice(get().bySession, sessionId);
      const messages = slice.messages.filter((m) => m.id !== rowId);
      if (messages.length === slice.messages.length) return;
      patch(sessionId, { messages });
    },

    onBufferReverted: (sessionId, rev) => {
      const slice = getSlice(get().bySession, sessionId);
      const messages = slice.messages.filter((m) => {
        const idx = rowBufferIndex(m);
        return idx !== null && idx < rev.committed_end;
      });
      debugTrace("buffer", "reverted", {
        sessionId,
        committedEnd: rev.committed_end,
        dropped: slice.messages.length - messages.length,
        ...overlayStats(messages),
      });
      const nextStart = Math.min(slice.bufferViewStart, rev.committed_end);
      patch(sessionId, {
        messages,
        bufferViewStart: nextStart,
        bufferViewEnd: rev.committed_end,
        committedBufferEnd: rev.committed_end,
        userDetailBefore: nextStart === 0 ? 0 : slice.userDetailBefore,
        shapeError: null,
        blockLogGrowth: true,
        turnEndNotice: null,
      });
    },

    loadRange: async (sessionId, start, end) => {
      const loaded = await useConnectionStore
        .getState()
        .sendRpc<BufferLoaded>("buffer/load", {
          start,
          end,
          session_id: sessionId,
        });
      get().onBufferLoaded(sessionId, loaded);
    },

    loadMoreHistory: (sessionId) => {
      const slice = getSlice(get().bySession, sessionId);
      if (slice.bufferViewStart <= 0) return;
      const end = slice.bufferViewStart;
      const start = Math.max(0, end - HISTORY_PAGE);
      if (start >= end) return;
      patch(sessionId, { loadingHistory: true });
      useConnectionStore
        .getState()
        .sendRpc<BufferLoaded>("buffer/load", {
          start,
          end,
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
      patch(sessionId, {
        messages: [],
        bufferViewStart: 0,
        bufferViewEnd: 0,
        committedBufferEnd: 0,
        userDetailBefore: 0,
        loadingHistory: false,
        shapeError: null,
        subagentBindings: {},
        blockLogGrowth: false,
        turnEndNotice: null,
      });
    },
  };
});

attachSiblingStores({ message: useMessageStore });
