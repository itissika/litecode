import { create } from "zustand";
import type { ChatRow } from "../api/adapter";
import {
  applyStreamEvent,
  bufferItemId,
  extractBufferIndex,
  isCompactCutRow,
  isEmptyItemShell,
  isStreamFailureEvent,
  isUserMessage,
  itemAuthorityId,
  itemPlainText,
  liveItemRowId,
  markFunctionCallsFailed,
  sealProjectionRow,
} from "../api/adapter";
import type {
  BufferItemNotification,
  BufferCompacted,
  BufferLoaded,
  Item,
  ResponseStreamEvent,
  SubagentBound,
} from "../api/types";
import { useConnectionStore, attachSiblingStores } from "./connectionStore";
import { useToastStore } from "./toastStore";

const HISTORY_PAGE = 40;

export interface MessageSlice {
  /** Projection rows — each row is an authority Item (live shell or sealed). */
  messages: ChatRow[];
  bufferViewStart: number;
  bufferViewEnd: number;
  committedBufferEnd: number;
  /**
   * Absolute user-detail count with buffer index `< bufferViewStart`.
   * From the latest `buffer/load` that extended (or set) the view start.
   */
  userDetailBefore: number;
  loadingHistory: boolean;
  /** Explicit error when a stream/buffer event cannot map to Item authority. */
  shapeError: string | null;
  /** `subagent_launch` call_id → child session id (live bind + buffer/load). */
  subagentBindings: Record<string, string>;
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

function findLastIndex<T>(arr: T[], predicate: (item: T) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (predicate(arr[i])) return i;
  }
  return -1;
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
    return aid === authId || m.id === liveItemRowId(authId);
  });
}

function reindexSealed(sessionId: string, row: ChatRow, index: number): ChatRow {
  return { ...row, id: bufferItemId(sessionId, index) };
}

/**
 * Compact does not rewrite details: first pass appends a checkpoint; a later
 * pass deletes the previous checkpoint (indices after it shift down by 1) and
 * appends a new one. Keep live rows. Do not wipe the materialized window.
 */
export function applyCompactSplice(
  sessionId: string,
  slice: MessageSlice,
  committedEnd: number,
): MessageSlice {
  const prevEnd = Math.max(slice.committedBufferEnd, slice.bufferViewEnd);
  const grew = committedEnd > prevEnd;

  const sealed: { index: number; row: ChatRow }[] = [];
  const transient: ChatRow[] = [];
  for (const row of slice.messages) {
    const index = extractBufferIndex(row.id);
    if (index === null) transient.push(row);
    else sealed.push({ index, row });
  }

  let viewStart = slice.bufferViewStart;
  let viewEnd = slice.bufferViewEnd;
  const localCp = sealed.find((entry) => isCompactCutRow(entry.row));

  let nextSealed = sealed;
  if (localCp) {
    const removed = localCp.index;
    nextSealed = sealed
      .filter((entry) => entry.index !== removed)
      .map((entry) =>
        entry.index > removed
          ? {
              index: entry.index - 1,
              row: reindexSealed(sessionId, entry.row, entry.index - 1),
            }
          : entry,
      );
    if (viewStart > removed) viewStart -= 1;
    if (viewEnd > removed) viewEnd -= 1;
  } else if (!grew && committedEnd === prevEnd && viewStart > 0) {
    nextSealed = sealed.map((entry) => ({
      index: entry.index - 1,
      row: reindexSealed(sessionId, entry.row, entry.index - 1),
    }));
    viewStart = Math.max(0, viewStart - 1);
    viewEnd = Math.max(0, viewEnd - 1);
  }

  nextSealed.sort((left, right) => left.index - right.index);
  const maxSealed = nextSealed.reduce((m, e) => Math.max(m, e.index + 1), 0);

  return {
    ...slice,
    messages: [...nextSealed.map((entry) => entry.row), ...transient],
    bufferViewStart: viewStart,
    bufferViewEnd: Math.max(viewEnd, maxSealed),
    committedBufferEnd: committedEnd,
    shapeError: null,
  };
}

function hasCheckpointAt(slice: MessageSlice, index: number): boolean {
  return slice.messages.some(
    (row) => isCompactCutRow(row) && extractBufferIndex(row.id) === index,
  );
}

/**
 * After buffer/item seal: place committed rows by `buffer_index`, keep live /
 * optimistic rows after them (stable relative order). Fixes stream arrival
 * order that diverged from transcript order without waiting for buffer/load.
 * Duplicate buffer ids (fail-closed mismatch push) keep insertion order via
 * stable sort.
 */
function orderSealedBeforeTransient(messages: ChatRow[]): ChatRow[] {
  const sealed: ChatRow[] = [];
  const transient: ChatRow[] = [];
  for (const row of messages) {
    if (extractBufferIndex(row.id) != null) sealed.push(row);
    else transient.push(row);
  }
  sealed.sort((a, b) => {
    const ai = extractBufferIndex(a.id) ?? 0;
    const bi = extractBufferIndex(b.id) ?? 0;
    return ai - bi;
  });
  return [...sealed, ...transient];
}

function findUnclaimedLive(
  transient: ChatRow[],
  committed: Item,
  claimed: Set<number>,
): number {
  const idx = findRowForSeal(transient, committed);
  if (idx < 0 || claimed.has(idx)) return -1;
  return idx;
}

function ingestBufferItems(
  set: (partial: Partial<MessageState>) => void,
  getState: () => MessageState,
  sessionId: string,
  items: BufferRowItem[],
  loadMeta?: { start: number; userDetailBefore: number },
): void {
  if (items.length === 0) return;

  const state = getState();
  const slice = getSlice(state.bySession, sessionId);

  const persisted = new Map<number, ChatRow>();
  const transient: ChatRow[] = [];
  for (const row of slice.messages) {
    const index = extractBufferIndex(row.id);
    if (index === null) transient.push(row);
    else persisted.set(index, row);
  }
  const hadPersisted = persisted.size > 0;
  const claimed = new Set<number>();

  for (const entry of items) {
    const bufferId = bufferItemId(sessionId, entry.bufferIndex);
    const liveIdx = findUnclaimedLive(transient, entry.item, claimed);
    if (liveIdx >= 0) {
      const { row, mismatch } = sealProjectionRow(
        transient[liveIdx],
        entry.item,
        bufferId,
        entry.kind,
      );
      if (!mismatch) {
        persisted.set(entry.bufferIndex, row);
        claimed.add(liveIdx);
        continue;
      }
    }

    const existing = persisted.get(entry.bufferIndex);
    if (existing) {
      const { row, mismatch } = sealProjectionRow(
        existing,
        entry.item,
        bufferId,
        entry.kind,
      );
      persisted.set(
        entry.bufferIndex,
        mismatch
          ? { id: bufferId, item: entry.item, kind: entry.kind, streaming: false }
          : row,
      );
      continue;
    }

    persisted.set(entry.bufferIndex, {
      id: bufferId,
      item: entry.item,
      kind: entry.kind,
      streaming: false,
    });
  }

  // Optimistic user-* has no authority id — still drop by text when the
  // committed window already contains that message.
  const loadedUserTexts = new Map<string, number>();
  for (const entry of items) {
    if (!isUserMessage(entry.item)) continue;
    const text = itemPlainText(entry.item);
    loadedUserTexts.set(text, (loadedUserTexts.get(text) ?? 0) + 1);
  }
  const remainingTransient: ChatRow[] = [];
  for (let i = transient.length - 1; i >= 0; i--) {
    if (claimed.has(i)) continue;
    const row = transient[i];
    if (isUserMessage(row.item)) {
      const text = itemPlainText(row.item);
      const count = loadedUserTexts.get(text) ?? 0;
      if (count > 0) {
        loadedUserTexts.set(text, count - 1);
        continue;
      }
    }
    remainingTransient.unshift(row);
  }

  const nextMessages = [
    ...[...persisted.entries()]
      .sort(([left], [right]) => left - right)
      .map(([, row]) => row),
    ...remainingTransient,
  ];

  const indices = items.map((i) => i.bufferIndex);
  const minIdx = Math.min(...indices);
  const maxIdx = Math.max(...indices) + 1;
  const nextStart = hadPersisted ? Math.min(slice.bufferViewStart, minIdx) : minIdx;

  // Baseline tracks the absolute user count before the current view start.
  // Refresh when this load is the first window or extends the view backward.
  let userDetailBefore = slice.userDetailBefore;
  if (loadMeta && (!hadPersisted || loadMeta.start < slice.bufferViewStart)) {
    userDetailBefore = loadMeta.userDetailBefore;
  }

  const nextSlice: MessageSlice = {
    ...slice,
    messages: nextMessages,
    bufferViewStart: nextStart,
    bufferViewEnd: Math.max(slice.bufferViewEnd, maxIdx),
    userDetailBefore,
  };

  const bySession = new Map(state.bySession);
  bySession.set(sessionId, nextSlice);
  set({ bySession });
}

interface MessageStore extends MessageState {
  onBufferLoaded: (sessionId: string, loaded: BufferLoaded) => void;
  onBufferItem: (sessionId: string, bi: BufferItemNotification) => void;
  onBufferReverted: (sessionId: string, rev: { session_id: string; committed_end: number }) => void;
  onBufferCompacted: (sessionId: string, compacted: BufferCompacted) => void;
  onSubagentBound: (sessionId: string, bound: SubagentBound) => void;

  applyStreamEvent: (
    sessionId: string,
    turnId: string,
    step: number,
    event: ResponseStreamEvent,
  ) => void;
  finalizeTurn: (sessionId: string, turnId: string) => void;

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
      const itemList: BufferRowItem[] = loaded.items.map((item, i) => ({
        bufferIndex: loaded.start + i,
        item,
        kind: loaded.kinds?.[i],
      }));
      ingestBufferItems(set, get, sessionId, itemList, {
        start: loaded.start,
        userDetailBefore: loaded.user_detail_before ?? 0,
      });
      const bindings = loaded.subagent_bindings ?? {};
      const slice = getSlice(get().bySession, sessionId);
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

    onBufferItem: (sessionId, bi) => {
      const bufferId = bufferItemId(sessionId, bi.buffer_index);
      const slice = getSlice(get().bySession, sessionId);

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

      if (isUserMessage(bi.item)) {
        if (slice.messages.some((row) => row.id === bufferId)) {
          patch(sessionId, { committedBufferEnd: bi.buffer_index + 1 });
          return;
        }

        const optimisticIdx = findLastIndex(
          slice.messages,
          (m) => m.id.startsWith("user-") && isUserMessage(m.item),
        );
        if (optimisticIdx >= 0) {
          const optimistic = slice.messages[optimisticIdx];
          const { row, mismatch } = sealProjectionRow(
            optimistic,
            bi.item,
            bufferId,
            bi.kind,
          );
          const next = [...slice.messages];
          next[optimisticIdx] = mismatch
            ? { id: bufferId, item: bi.item, kind: bi.kind, streaming: false }
            : row;
          patch(sessionId, {
            messages: orderSealedBeforeTransient(next),
            committedBufferEnd: bi.buffer_index + 1,
          });
          return;
        }

        patch(sessionId, {
          messages: orderSealedBeforeTransient([
            ...slice.messages,
            { id: bufferId, item: bi.item, kind: bi.kind },
          ]),
          committedBufferEnd: bi.buffer_index + 1,
        });
        return;
      }

      // Seal the same Item slot: find live row by (authority id + type), else by buffer index id.
      // Type matters: function_call and function_call_output share call_id and must not collide.
      const messages = [...slice.messages];
      let sealIdx = findRowForSeal(messages, bi.item);
      if (sealIdx < 0) {
        sealIdx = messages.findIndex((m) => m.id === bufferId);
      }

      if (sealIdx >= 0) {
        const { row, mismatch } = sealProjectionRow(
          messages[sealIdx],
          bi.item,
          bufferId,
          bi.kind,
        );
        if (mismatch) {
          // H3 fail-closed: never overwrite a differently-typed (or id-mismatched) slot.
          reportShapeError(patch, sessionId, mismatch);
          messages.push({ id: bufferId, item: bi.item, kind: bi.kind, streaming: false });
        } else {
          messages[sealIdx] = row;
        }
      } else {
        messages.push({ id: bufferId, item: bi.item, kind: bi.kind, streaming: false });
      }

      patch(sessionId, {
        messages: orderSealedBeforeTransient(messages),
        committedBufferEnd: bi.buffer_index + 1,
      });
    },

    applyStreamEvent: (sessionId, _turnId, _step, event) => {
      const slice = getSlice(get().bySession, sessionId);

      // A turn-level failure (response.failed / error) invalidates every
      // in-flight function_call — no half-streamed call may stay "in_progress".
      // `response.incomplete` is a seal terminal, not a failure: live rows wait
      // for buffer/item.
      if (isStreamFailureEvent(event)) {
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
      if (existingIdx >= 0 && extractBufferIndex(slice.messages[existingIdx].id) != null) {
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
          // Keep buffer id if already sealed id somehow; otherwise live id.
          id: extractBufferIndex(messages[idx].id) != null ? messages[idx].id : rowId,
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
      patch(sessionId, { messages, shapeError: null });
    },

    finalizeTurn: (sessionId, _turnId) => {
      const slice = getSlice(get().bySession, sessionId);
      const messages = slice.messages
        .map((m) => {
          if (!m.streaming) return m;
          if (isEmptyItemShell(m.item) && m.id.startsWith("live-")) {
            return null;
          }
          return { ...m, streaming: false };
        })
        .filter((m): m is ChatRow => m != null);
      patch(sessionId, { messages });
    },

    pushUserMessage: (sessionId, row) => {
      const slice = getSlice(get().bySession, sessionId);
      patch(sessionId, { messages: [...slice.messages, row] });
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
        if (m.id.startsWith("live-")) return false;
        const idx = extractBufferIndex(m.id);
        return idx === null || idx < rev.committed_end;
      });
      patch(sessionId, {
        messages,
        bufferViewEnd: rev.committed_end,
        committedBufferEnd: rev.committed_end,
        shapeError: null,
      });
    },

    onBufferCompacted: (sessionId, compacted) => {
      const end = compacted.committed_end;
      const slice = getSlice(get().bySession, sessionId);
      const spliced = applyCompactSplice(sessionId, slice, end);
      patch(sessionId, spliced);
      if (end === 0) return;

      const checkpointIndex = end - 1;
      if (hasCheckpointAt(spliced, checkpointIndex)) return;

      useConnectionStore
        .getState()
        .sendRpc<BufferLoaded>("buffer/load", {
          start: checkpointIndex,
          end,
          session_id: sessionId,
        })
        .then((loaded) => {
          get().onBufferLoaded(sessionId, loaded);
        })
        .catch((error: unknown) => {
          useToastStore.getState().showToast(
            error instanceof Error ? error.message : "Failed to load compact checkpoint",
            "error",
          );
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
      });
    },
  };
});

attachSiblingStores({ message: useMessageStore });
