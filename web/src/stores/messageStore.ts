import { create } from "zustand";
import type { ChatRow } from "../api/adapter";
import {
  applyStreamEvent,
  bufferItemId,
  extractBufferIndex,
  isCompactCutRow,
  isStreamFailureEvent,
  isUserMessage,
  itemAuthorityId,
  itemPlainText,
  liveItemRowId,
  markFunctionCallsFailed,
  sealProjectionRow,
  wireRowKind,
} from "../api/adapter";
import type {
  BufferItemNotification,
  BufferCompacted,
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
    if (extractBufferIndex(row.id) == null) {
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
    return aid === authId || m.id === liveItemRowId(authId);
  });
}

function reindexSealed(sessionId: string, row: ChatRow, index: number): ChatRow {
  return { ...row, id: bufferItemId(sessionId, index) };
}

/**
 * Compact does not rewrite details. A later pass deletes the previous
 * checkpoint (indices after it shift down by 1) and appends a new one.
 * Only splice when the old cut is in the local window — never blind-shift.
 */
export function applyCompactSplice(
  sessionId: string,
  slice: MessageSlice,
  committedEnd: number,
): MessageSlice {
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
  }

  nextSealed.sort((left, right) => left.index - right.index);
  const maxSealed = nextSealed.reduce((m, e) => Math.max(m, e.index + 1), 0);

  return {
    ...slice,
    messages: orderProjection(
      [...nextSealed.map((entry) => entry.row), ...transient],
      committedEnd,
    ),
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

function projectionSortKey(row: ChatRow, committedEnd: number): number {
  const index = extractBufferIndex(row.id);
  if (index != null) return index;
  if (row.id.startsWith("user-")) return committedEnd;
  return Number.POSITIVE_INFINITY;
}

/** Sealed by buffer_index; optimistic user occupies committedEnd; live after that. */
function orderProjection(messages: ChatRow[], committedEnd: number): ChatRow[] {
  return [...messages].sort((left, right) => {
    return projectionSortKey(left, committedEnd) - projectionSortKey(right, committedEnd);
  });
}

function compactProjectionMismatch(slice: MessageSlice, committedEnd: number): boolean {
  let cuts = 0;
  for (const row of slice.messages) {
    const index = extractBufferIndex(row.id);
    if (index == null) continue;
    if (index >= committedEnd) return true;
    if (isCompactCutRow(row)) cuts += 1;
  }
  return cuts > 1;
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

function isDetailUserEntry(item: Item, kind?: string): boolean {
  return wireRowKind(kind) !== "compact_checkpoint" && isUserMessage(item);
}

function isOptimisticUserShell(row: ChatRow): boolean {
  return (
    row.id.startsWith("user-") &&
    isUserMessage(row.item) &&
    !isCompactCutRow(row)
  );
}

function contiguousCommittedEnd(
  prev: number,
  persisted: Map<number, ChatRow>,
  hadPersisted: boolean,
  incomingMaxExcl: number,
): number {
  if (!hadPersisted) return Math.max(prev, incomingMaxExcl);
  let end = prev;
  while (persisted.has(end)) end += 1;
  return end;
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

  const persisted = new Map<number, ChatRow>();
  const transient: ChatRow[] = [];
  for (const row of slice.messages) {
    const index = extractBufferIndex(row.id);
    if (index === null) transient.push(row);
    else persisted.set(index, row);
  }
  const hadPersisted = persisted.size > 0;
  const claimed = new Set<number>();
  let shapeError = slice.shapeError;

  for (const entry of incoming) {
    const kind = wireRowKind(entry.kind);
    const bufferId = bufferItemId(sessionId, entry.bufferIndex);
    const liveIdx =
      kind === "compact_checkpoint"
        ? -1
        : findUnclaimedLive(transient, entry.item, claimed);
    if (liveIdx >= 0) {
      const { row, mismatch } = sealProjectionRow(
        transient[liveIdx],
        entry.item,
        bufferId,
        kind,
      );
      if (!mismatch) {
        persisted.set(entry.bufferIndex, row);
        claimed.add(liveIdx);
        continue;
      }
      shapeError = mismatch;
      useToastStore.getState().showToast(mismatch, "error");
    }

    const existing = persisted.get(entry.bufferIndex);
    if (existing) {
      const { row, mismatch } = sealProjectionRow(
        existing,
        entry.item,
        bufferId,
        kind,
      );
      if (mismatch) {
        shapeError = mismatch;
        useToastStore.getState().showToast(mismatch, "error");
        persisted.set(entry.bufferIndex, {
          id: bufferId,
          item: entry.item,
          kind,
          streaming: false,
        });
      } else {
        persisted.set(entry.bufferIndex, row);
      }
      continue;
    }

    persisted.set(entry.bufferIndex, {
      id: bufferId,
      item: entry.item,
      kind,
      streaming: false,
    });
  }

  const loadedUserTexts = new Map<string, number>();
  for (const entry of incoming) {
    if (!isDetailUserEntry(entry.item, entry.kind)) continue;
    const text = itemPlainText(entry.item);
    loadedUserTexts.set(text, (loadedUserTexts.get(text) ?? 0) + 1);
  }
  const remainingTransient: ChatRow[] = [];
  for (let i = transient.length - 1; i >= 0; i--) {
    if (claimed.has(i)) continue;
    const row = transient[i];
    if (isOptimisticUserShell(row)) {
      const text = itemPlainText(row.item);
      const count = loadedUserTexts.get(text) ?? 0;
      if (count > 0) {
        loadedUserTexts.set(text, count - 1);
        continue;
      }
    }
    remainingTransient.unshift(row);
  }

  const dropIdleLive = loadMeta?.dropIdleLive === true;
  const keptTransient = dropIdleLive
    ? remainingTransient.filter((row) => row.id.startsWith("user-"))
    : remainingTransient;

  const indices = incoming.map((i) => i.bufferIndex);
  const minIdx = Math.min(...indices);
  const maxIdx = Math.max(...indices) + 1;
  const nextCommitted = slice.blockLogGrowth
    ? slice.committedBufferEnd
    : contiguousCommittedEnd(
        slice.committedBufferEnd,
        persisted,
        hadPersisted,
        maxIdx,
      );
  const nextMessages = orderProjection(
    [...persisted.values(), ...keptTransient],
    nextCommitted,
  );

  const nextStart = hadPersisted ? Math.min(slice.bufferViewStart, minIdx) : minIdx;

  let userDetailBefore = slice.userDetailBefore;
  if (loadMeta && (!hadPersisted || loadMeta.start < slice.bufferViewStart)) {
    userDetailBefore = loadMeta.userDetailBefore;
  }

  const nextSlice: MessageSlice = {
    ...slice,
    messages: nextMessages,
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
  onBufferCompacted: (sessionId: string, compacted: BufferCompacted) => void;
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
      const itemList: BufferRowItem[] = loaded.items.map((item, i) => ({
        bufferIndex: loaded.start + i,
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
      const pendingOptimistic = slice.messages.some((row) =>
        row.id.startsWith("user-"),
      );
      if (
        bi.buffer_index >
        slice.committedBufferEnd + (pendingOptimistic ? 1 : 0)
      ) {
        debugTrace("buffer", "item.dropped_gap", itemMeta);
        return;
      }
      const append = bi.buffer_index === slice.committedBufferEnd;
      if (append && slice.blockLogGrowth) {
        debugTrace("buffer", "item.dropped_blocked", itemMeta);
        return;
      }

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
      if (existingIdx >= 0 && extractBufferIndex(slice.messages[existingIdx].id) != null) {
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
      patch(sessionId, {
        messages: orderProjection(messages, slice.committedBufferEnd),
        shapeError: null,
      });
    },

    finalizeTurn: (sessionId, _turnId) => {
      const slice = getSlice(get().bySession, sessionId);
      let droppedLive = 0;
      const messages = slice.messages.filter((m) => {
        if (m.id.startsWith("live-")) {
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
        messages: orderProjection(messages, slice.committedBufferEnd),
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
        messages: orderProjection([...slice.messages, row], slice.committedBufferEnd),
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
        const idx = extractBufferIndex(m.id);
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

    onBufferCompacted: (sessionId, compacted) => {
      const end = compacted.committed_end;
      const slice = getSlice(get().bySession, sessionId);
      const spliced = applyCompactSplice(sessionId, slice, end);
      patch(sessionId, spliced);
      if (end === 0) return;

      const loadWindow = () => {
        const start = Math.min(spliced.bufferViewStart, end);
        if (start >= end) return;
        void get().loadRange(sessionId, start, end);
      };

      if (compactProjectionMismatch(spliced, end)) {
        loadWindow();
        return;
      }

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
          const after = getSlice(get().bySession, sessionId);
          if (
            !hasCheckpointAt(after, checkpointIndex) ||
            compactProjectionMismatch(after, end)
          ) {
            const start = Math.min(after.bufferViewStart, end);
            if (start < end) void get().loadRange(sessionId, start, end);
          }
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
        blockLogGrowth: false,
        turnEndNotice: null,
      });
    },
  };
});

attachSiblingStores({ message: useMessageStore });
