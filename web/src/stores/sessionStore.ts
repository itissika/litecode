import { create } from "zustand";
import type {
  WireServerHello,
  SessionSnapshot,
  SessionInfo,
  SessionFolder,
  SessionLifecycle,
  SessionList,
  OperationResult,
  OperationKind,
  PrimaryAgentInfo,
  ModelInfo,
  BufferLoaded,
  ThinkingTier,
  ContextMode,
} from "../api/types";
import {
  useConnectionStore,
  attachSiblingStores,
} from "./connectionStore";
import { closePanel } from "../dockview/workbench/commands";
import { useBashStore } from "./bashStore";
import { useToastStore } from "./toastStore";
import { useTurnStore } from "./turnStore";
import { noteSessionFinished } from "./unreadStore";
import { useMessageStore } from "./messageStore";
import { openSessionPanel } from "../lib/sessionPanelNav";
import { deleteRejectedMessage, rpcErrorCode } from "../lib/sessionErrors";
import { noteSessionListSettled } from "../lib/centreSettle";
import i18n from "../i18n";

let listInFlight = false;
/** A request arrived while one was in flight → re-pull once after it settles. */
let listPending = false;
/** Test hook: reset the session/list in-flight guard. */
export function resetListSessionsInFlight(): void {
  listInFlight = false;
  listPending = false;
}

/** One delete per id at a time. The row stays visible until the commit, so a
 *  second click must not send a duplicate `session/delete` — the server treats
 *  an already-gone id as an error, not a no-op. */
const deleteInFlight = new Set<string>();
/** Test hook: reset the delete in-flight guard. */
export function resetDeleteSessionInFlight(): void {
  deleteInFlight.clear();
}

/** Ordering granularity: one minute — the resolution of the sidebar's own
 *  relative label ("1m", "2m", …). `updated_at` moves on every streamed flush
 *  (the runtime coalesces at 80ms), so comparing raw milliseconds makes two
 *  sessions that stream in parallel swap rows on every flush. Inside a minute
 *  they tie, the stable sort keeps the row that was already higher, and the
 *  order never contradicts the labels the user is reading. */
const ORDER_BUCKET_MS = 60_000;

function orderBucket(updatedAt: number | null | undefined): number {
  return Math.floor((updatedAt ?? 0) / ORDER_BUCKET_MS);
}

/**
 * Most-recently-updated first, at minute granularity. The backend returns the
 * list pre-sorted, but live lifecycle events mutate it in place; re-sort so a
 * session a minute (or more) newer than the rest bubbles to the top. Stable
 * sort — sessions in the same minute keep their relative order.
 */
function sortSessions(sessions: SessionInfo[]): SessionInfo[] {
  return [...sessions].sort(
    (a, b) => orderBucket(b.updated_at) - orderBucket(a.updated_at),
  );
}

function sortFolders(folders: SessionFolder[]): SessionFolder[] {
  return [...folders].sort((a, b) => a.sort_order - b.sort_order);
}

function sessionControlsLocked(sessionId: string): boolean {
  const slice = useTurnStore.getState().byId.get(sessionId);
  const run = slice?.runState;
  return (
    run === "running" || run === "cancelling" || slice?.manualCompacting === true
  );
}

interface SessionSlice {
  /** Sticky primary agent id — hydrated from snapshot.agent_id. */
  activePrimary: string;
  pendingPrimaryId: string | null;
  /** Sticky composite model ref (`{provider_id}/{model_id}`) from the snapshot. */
  modelId: string | null;
  /** Effective wire id from snapshot.api_model_id. */
  apiModelId: string;
  /** Optional display label from snapshot. */
  label: string;
  /** Platform thinking tier — hydrated from snapshot.thinking_tier. */
  thinkingTier: ThinkingTier;
  /** Platform context mode — hydrated from snapshot.context_mode. */
  contextMode: ContextMode;
  pendingThinkingTier: ThinkingTier | null;
  pendingContextMode: ContextMode | null;
  /** Highest file-snapshot stem with a nonempty patch; null = hide Revert files. */
  maxFileRevertSeq: number | null;
}

function emptySlice(): SessionSlice {
  return {
    activePrimary: "default",
    pendingPrimaryId: null,
    modelId: null,
    apiModelId: "",
    label: "",
    thinkingTier: "medium",
    contextMode: "standard",
    pendingThinkingTier: null,
    pendingContextMode: null,
    maxFileRevertSeq: null,
  };
}

function getSlice(
  byId: Map<string, SessionSlice>,
  sessionId: string,
): SessionSlice {
  let slice = byId.get(sessionId);
  if (!slice) {
    slice = emptySlice();
    byId.set(sessionId, slice);
  }
  return slice;
}

interface SessionState {
  byId: Map<string, SessionSlice>;

  // Global fields (not per-session)
  activePrimary: string;
  pendingPrimaryId: string | null;
  primaryAgents: PrimaryAgentInfo[];
  project: string;
  sessions: SessionInfo[];
  /** Sidebar folders, ordered by `sort_order`. */
  folders: SessionFolder[];
  sessionsLoading: boolean;
  sessionListError: string | null;
  pendingSessionOp: OperationKind | null;
  statusMessage: string | null;
  showSessionList: boolean;
  availableModels: ModelInfo[];
  /** WP-086: ids seen in a `deleted` lifecycle event (tool cards / roster / counts). */
  deletedSessionIds: Record<string, true>;
}

interface SessionStore extends SessionState {
  // Server notification handlers
  onHello: (hello: WireServerHello) => void;
  onModelsChanged: (models: ModelInfo[]) => void;
  applySnapshot: (snap: SessionSnapshot) => void;
  onSessionList: (sessions: SessionInfo[], folders?: SessionFolder[]) => void;
  onSessionLifecycle: (params: SessionLifecycle) => void;
  onSessionAttached: (params: { session_id: string; turn: any }) => void;
  onOperationResult: (op: OperationResult) => void;

  // RPC methods
  newSession: () => void;
  deleteSession: (id: string) => void;
  /** WP-086: stop a subagent child via `agent/cancel`; `cancelled:false` → neutral toast. */
  stopSubagent: (childId: string) => void;
  listSessions: () => void;
  setPrimary: (sessionId: string, agentId: string) => void;
  setModel: (sessionId: string, modelId: string) => void;
  setThinkingTier: (sessionId: string, tier: ThinkingTier) => void;
  setContextMode: (sessionId: string, mode: ContextMode) => void;

  // Display-only sidebar layout
  renameSession: (id: string, title: string | null) => void;
  moveSession: (id: string, folderId: string | null) => void;
  createFolder: (name: string) => void;
  renameFolder: (id: string, name: string) => void;
  deleteFolder: (id: string) => void;

  // UI actions
  setShowSessionList: (open: boolean) => void;
}

export const useSessionStore = create<SessionStore>((set, get) => {
  function patch(sessionId: string, update: Partial<SessionSlice>): void {
    const byId = new Map(get().byId);
    const slice = { ...getSlice(byId, sessionId), ...update };
    byId.set(sessionId, slice);
    set({ byId });
  }

  function applySnapshot(snap: SessionSnapshot): void {
    if (!snap.session_id) {
      return;
    }

    const sessionId = snap.session_id;

    patch(sessionId, {
      activePrimary: snap.agent_id || "default",
      pendingPrimaryId: null,
      modelId: snap.model_id ?? null,
      apiModelId: snap.api_model_id ?? "",
      label: snap.label ?? "",
      thinkingTier: snap.thinking_tier ?? "medium",
      contextMode: snap.context_mode ?? "standard",
      pendingThinkingTier: null,
      pendingContextMode: null,
      maxFileRevertSeq:
        snap.max_file_revert_seq === undefined || snap.max_file_revert_seq === null
          ? null
          : snap.max_file_revert_seq,
    });

    // Snapshot turn is hydrate-only. `turn: null` (compact / idle session) must
    // not force idle — that belongs to turn_finished. A compact snapshot that
    // lands after the next turn has started would otherwise drop stream deltas.
    if (snap.turn) {
      useTurnStore.getState().applySnapshotTurn(sessionId, snap.turn);
    }
    useTurnStore.getState().applyLlmReconnect(sessionId, snap.llm_reconnect);
    useTurnStore.getState().applySnapshotMeter(sessionId, snap);
    if (snap.bash) {
      useBashStore.getState().applySnapshot(sessionId, snap.bash);
    }
    useMessageStore
      .getState()
      .applySubagentBindingStates(sessionId, snap.subagent_binding_states);
    if (snap.buffer.next_seq === 0) {
      useMessageStore.getState().onBufferLoaded(sessionId, {
        session_id: sessionId,
        from_seq: 0,
        to_seq: 0,
        events: [],
      });
      set({ pendingSessionOp: null });
      return;
    }

    // Cold-start history only. Growth after the window exists is buffer/item.
    const msgSlice = useMessageStore.getState().bySession.get(sessionId);
    const needsInitialLoad =
      !msgSlice || (msgSlice.toSeq === 0 && msgSlice.messages.length === 0);

    if (needsInitialLoad) {
      const toSeq = snap.buffer.next_seq;
      const fromSeq = Math.max(0, toSeq - 40);
      useConnectionStore
        .getState()
        .sendRpc<BufferLoaded>("buffer/load", {
          from_seq: fromSeq,
          to_seq: toSeq,
          session_id: sessionId,
        })
        .then((loaded) => {
          useMessageStore.getState().onBufferLoaded(sessionId, loaded);
        })
        .catch(() => {
          set({ statusMessage: "Failed to load transcript buffer" });
        });
    } else if (
      msgSlice &&
      !msgSlice.blockLogGrowth &&
      msgSlice.toSeq < snap.buffer.next_seq
    ) {
      // Retained window that lags the server: a collapsed roster card keeps the
      // child's slices (P6) and a reconnect keeps every slice. Append the missing
      // tail — `[toSeq, next_seq)` is exactly the gap — instead of cold-starting.
      // Repeated snapshots may re-fetch an already-covered range; `upsertEvents`
      // keys by seq, so that is idempotent. A reverted window blocks growth, so a
      // stale snapshot cannot pull the discarded rows back.
      useMessageStore
        .getState()
        .loadRange(sessionId, msgSlice.toSeq, snap.buffer.next_seq)
        .catch(() => {});
    }

    set({ pendingSessionOp: null });
  }

  function onOperationResult(op: OperationResult): void {
    if (op.ok) {
      if (op.op === "new_session") {
        applySnapshot(op.snapshot);
        set({ pendingSessionOp: null, statusMessage: null });
      }
      // delete_session: list / lifecycle are authoritative. Do not hydrate a
      // snapshot for a session that no longer exists.
      if (
        op.op === "set_model" ||
        op.op === "set_active_primary" ||
        op.op === "set_thinking_tier" ||
        op.op === "set_context_mode"
      ) {
        applySnapshot(op.snapshot);
      }
      if (op.op === "revert_to_user_anchor") {
        applySnapshot(op.snapshot);
        // `buffer/reverted` normally arrives before the operation result. If
        // that notification was lost during a socket hiccup, the snapshot is
        // still authoritative and must trim an already-loaded local window.
        // A loaded window past the surviving tail (`last_seq`) still holds rows
        // the revert deleted. `next_seq` is the allocator high-water and does
        // not move back, so it cannot be the threshold.
        const local = useMessageStore
          .getState()
          .bySession.get(op.snapshot.session_id);
        if (local && local.toSeq > op.snapshot.buffer.last_seq + 1) {
          useTurnStore.getState().onTranscriptReverted(op.snapshot.session_id);
          useMessageStore.getState().onBufferReverted(op.snapshot.session_id, {
            session_id: op.snapshot.session_id,
            last_seq: op.snapshot.buffer.last_seq,
            next_seq: op.snapshot.buffer.next_seq,
          });
        }
        useToastStore.getState().showToast("Transcript reverted", "success");
      }
      if (op.op === "revert_files") {
        useToastStore.getState().showToast("Files reverted", "success");
      }
      if (op.op === "compact_session") {
        applySnapshot(op.snapshot);
        useToastStore.getState().showToast("Context compacted", "success");
      }
      return;
    }

    const errMsg = op.error?.message ?? op.error?.code ?? "Operation failed";

    if (op.op === "new_session") {
      useToastStore.getState().showToast(errMsg, "error");
      set({
        pendingSessionOp: null,
        statusMessage: errMsg,
      });
      return;
    }

    if (op.op === "delete_session") {
      // RPC catch already toasts; re-pull so the row follows the server's state.
      get().listSessions();
      return;
    }

    if (op.op === "compact_session") {
      applySnapshot(op.snapshot);
      useToastStore.getState().showToast(errMsg, "error");
      return;
    }

    if (
      op.op === "set_model" ||
      op.op === "set_active_primary" ||
      op.op === "set_thinking_tier" ||
      op.op === "set_context_mode"
    ) {
      useToastStore.getState().showToast(errMsg, "error");
      // Re-hydrate from error snapshot when present (clears pendingPrimaryId).
      if (op.snapshot?.session_id) {
        applySnapshot(op.snapshot);
      }
      return;
    }

    if (op.op === "revert_to_user_anchor" || op.op === "revert_files") {
      useToastStore.getState().showToast(errMsg, "error");
    }
  }

  return {
    byId: new Map(),
    activePrimary: "default",
    pendingPrimaryId: null,
    primaryAgents: [],
    project: "",
    sessions: [],
    folders: [],
    sessionsLoading: false,
    sessionListError: null,
    pendingSessionOp: null,
    statusMessage: null,
    showSessionList: false,
    availableModels: [],
    deletedSessionIds: {},

    onHello: (hello: WireServerHello) => {
      // server/hello carries only ACTIVE models (provider holds a credential);
      // the catalog itself is backend-owned and never edited from the Web app.
      set((s) => ({
        activePrimary: hello.active_primary ?? s.activePrimary,
        primaryAgents: hello.primary_agents ?? s.primaryAgents,
        pendingPrimaryId: null,
        availableModels: hello.models ?? [],
        project: hello.project || s.project,
      }));
    },

    onModelsChanged: (models: ModelInfo[]) => {
      // Credential change: the same projection the handshake uses, pushed live
      // so an open model switcher never shows a stale list.
      set({ availableModels: models });
    },

    applySnapshot,

    onSessionList: (sessions: SessionInfo[], folders?: SessionFolder[]) => {
      // A `session/list` snapshot can be built before a delete and arrive
      // after it (the fast path runs off the serial loop). Drop ids already
      // confirmed deleted so a late snapshot cannot resurrect a row.
      const deleted = get().deletedSessionIds;
      set({
        sessions: sortSessions(sessions.filter((s) => !deleted[s.id])),
        ...(folders ? { folders: sortFolders(folders) } : {}),
        sessionsLoading: false,
        sessionListError: null,
      });
    },

    onSessionLifecycle: (params) => {
      const {
        session_id,
        event,
        turn,
        preview,
        assistant_preview,
        updated_at,
        step_kind,
        project,
        agent_id,
        parent_session_id,
        parent_call_id,
        responsibility,
        reason,
        status,
      } = params;
      // Wire key on the event is `cancelled_by`; stored as last_turn_cancelled_by.
      const hasCancelledBy = "cancelled_by" in params;
      const cancelledBy = params.cancelled_by ?? null;
      // A root settling from running to idle while its transcript is hidden
      // earns the sidebar's unread dot. Requiring the transition (rather than
      // the event alone) keeps a replayed idle update from re-marking a dot the
      // user already consumed.
      const maySettle =
        event === "turn_finished" || event === "status_changed";
      const wasRunning =
        maySettle &&
        (get().sessions.find((s) => s.id === session_id)?.running ?? false);
      set((state) => {
        const exists = state.sessions.some((s) => s.id === session_id);
        if (event === "deleted") {
          useConnectionStore.getState().unsubscribeSession(session_id);
          closePanel(`agent-${session_id}`);
          closePanel(`subagent-${session_id}`);
          const deletedSessionIds = {
            ...state.deletedSessionIds,
            [session_id]: true as const,
          };
          return exists
            ? {
                deletedSessionIds,
                sessions: sortSessions(
                  state.sessions.filter((s) => s.id !== session_id),
                ),
              }
            : { deletedSessionIds };
        }
        // Late events for an id already confirmed deleted must not upsert it
        // back into the sidebar.
        if (state.deletedSessionIds[session_id]) return {};
        if (event === "status_changed") {
          // WP-086: aggregate status only; turn / step state untouched.
          if (!status || !exists) return {};
          const sessions = state.sessions.map((s) =>
            s.id === session_id
              ? { ...s, status, running: status !== "idle" }
              : s,
          );
          return { sessions: sortSessions(sessions) };
        }
        // turn_started / turn_updated / turn_finished may arrive for a session
        // that is not yet in the local list (e.g. a push arrived before the
        // initial pull completed). Upsert it so the live status is not dropped.
        // This row carries live state only: title / folder / preview are list
        // projection facts, and visibility stays the backend's call.
        if (!exists) {
          if (event === "turn_finished") {
            useTurnStore
              .getState()
              .onLifecycleTurnFinished(session_id, turn?.turn_id);
          }
          const fresh: SessionInfo = {
            id: session_id,
            project: project ?? "",
            updated_at: updated_at ?? Date.now(),
            preview: preview ?? "",
            assistant_preview: assistant_preview ?? undefined,
            title: null,
            folder_id: null,
            label: "",
            running: status ? status !== "idle" : event !== "turn_finished",
            ...(status ? { status } : {}),
            turn: turn ?? null,
            agent_id: agent_id ?? "",
            model_id: null,
            api_model_id: "",
            step_kinds: step_kind ? [step_kind] : [],
            // A subagent child arrives on the lifecycle feed too (the broadcast
            // is unfiltered), and the sidebar must not list it as a root — so
            // carry the parent ids the `created` event carries.
            parent_session_id: parent_session_id ?? null,
            parent_call_id: parent_call_id ?? null,
            responsibility: responsibility ?? "",
            last_turn_reason: event === "turn_finished" ? reason : undefined,
            last_turn_cancelled_by:
              event === "turn_finished" && hasCancelledBy
                ? cancelledBy
                : undefined,
          };
          return { sessions: sortSessions([...state.sessions, fresh]) };
        }
        switch (event) {
          case "preview_updated": {
            const sessions = state.sessions.map((s) =>
              s.id === session_id
                ? {
                    ...s,
                    preview: preview ?? s.preview,
                    assistant_preview: assistant_preview ?? s.assistant_preview,
                    updated_at: updated_at ?? s.updated_at,
                  }
                : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          case "turn_step": {
            const sessions = state.sessions.map((s) =>
              s.id === session_id
                ? {
                    ...s,
                    running: true,
                    turn: turn ?? s.turn,
                    step_kinds: step_kind
                      ? [...(s.step_kinds ?? []), step_kind]
                      : (s.step_kinds ?? []),
                  }
                : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          case "turn_started": {
            // New turn: wipe the previous turn's accumulated step kinds.
            const sessions = state.sessions.map((s) =>
              s.id === session_id
                ? {
                    ...s,
                    running: true,
                    status: status ?? ("running" as const),
                    turn: turn ?? s.turn,
                    step_kinds: [],
                    last_turn_reason: undefined,
                    last_turn_cancelled_by: undefined,
                  }
                : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          case "turn_updated": {
            const sessions = state.sessions.map((s) =>
              s.id === session_id
                ? {
                    ...s,
                    running: true,
                    status:
                      turn?.phase === "cancelling"
                        ? ("stopping" as const)
                        : ("running" as const),
                    turn: turn ?? s.turn,
                  }
                : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          case "turn_finished": {
            // Keep `step_kinds` for the recap; the UI holds it for a while then
            // transitions back to idle on its own timer. Cleared on next start.
            useTurnStore
              .getState()
              .onLifecycleTurnFinished(session_id, turn?.turn_id);
            const sessions = state.sessions.map((s) =>
              s.id === session_id
                ? {
                    ...s,
                    // WP-086: raw status (e.g. running_with_subagent) wins.
                    running: status ? status !== "idle" : false,
                    status: status ?? ("idle" as const),
                    turn: null,
                    last_turn_reason: reason ?? s.last_turn_reason,
                    // In lockstep with reason: a new reason without the field
                    // must not inherit the previous turn's initiator.
                    last_turn_cancelled_by: hasCancelledBy
                      ? cancelledBy
                      : reason !== undefined
                        ? undefined
                        : s.last_turn_cancelled_by,
                  }
                : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          default:
            return {};
        }
      });
      if (wasRunning) {
        const settled = get().sessions.find((s) => s.id === session_id);
        if (settled && !settled.running && !settled.parent_session_id) {
          noteSessionFinished(session_id);
        }
      }
    },

    onSessionAttached: (params) => {
      const { session_id, turn } = params;
      set((state) => {
        if (!state.sessions.some((s) => s.id === session_id)) return {};
        const sessions = state.sessions.map((s) =>
          s.id === session_id ? { ...s, running: true, turn } : s,
        );
        return { sessions: sortSessions(sessions) };
      });
    },

    onOperationResult,

    newSession: () => {
      const { pendingSessionOp } = get();
      if (pendingSessionOp !== null) return;
      set({
        pendingSessionOp: "new_session",
        showSessionList: false,
        statusMessage: "Starting new session…",
      });
      useConnectionStore
        .getState()
        .sendRpc<{ session_id: string }>("session/new")
        .then((result) => {
          set({ pendingSessionOp: null, statusMessage: null });
          if (result.session_id) {
            openSessionPanel(result.session_id);
          }
        })
        .catch(() => {
          set({
            pendingSessionOp: null,
            statusMessage: "Failed to create new session",
          });
        });
    },

    deleteSession: (id: string) => {
      // Product gate: only idle sessions can be deleted. UI also disables the
      // button; this is the sole store-side check (no pendingSessionOp lock).
      const target = get().sessions.find((s) => s.id === id);
      if (
        target?.running ||
        target?.status === "running_with_subagent" ||
        target?.status === "stopping"
      ) {
        return;
      }

      if (deleteInFlight.has(id)) return;
      deleteInFlight.add(id);

      useConnectionStore
        .getState()
        .sendRpc("session/delete", { id })
        .then(() => {
          // Confirmed by the server. Remove now — not before — so a rejected
          // delete never makes the row disappear and come back. Tombstone the
          // id so a stale list snapshot cannot re-add it.
          set((state) => ({
            deletedSessionIds: {
              ...state.deletedSessionIds,
              [id]: true as const,
            },
            sessions: sortSessions(state.sessions.filter((s) => s.id !== id)),
          }));
          useConnectionStore.getState().unsubscribeSession(id);
          closePanel(`agent-${id}`);
        })
        .catch((err: unknown) => {
          const code = rpcErrorCode(err);
          useToastStore
            .getState()
            .showToast(
              code ? deleteRejectedMessage(err) : "Failed to delete session",
              "error",
            );
          get().listSessions();
        })
        .finally(() => {
          deleteInFlight.delete(id);
        });
    },

    stopSubagent: (childId: string) => {
      useConnectionStore
        .getState()
        .sendRpc<{ cancelled?: boolean }>("agent/cancel", {
          session_id: childId,
        })
        .then((res) => {
          if (res?.cancelled === false) {
            useToastStore
              .getState()
              .showToast(i18n.t("session.noRunningTurn"), "info");
          }
        })
        .catch(() => {
          useToastStore.getState().showToast(i18n.t("session.stopFailed"), "error");
        });
    },

    listSessions: () => {
      // One request at a time: the connected transition, SessionList and the
      // Workers panel may all ask on the same tick (WP-086 reconnect dedupe).
      // A call during flight is coalesced into ONE follow-up pull, so a
      // create/delete that landed mid-request is never missed (WP-093).
      if (listInFlight) {
        listPending = true;
        return;
      }
      listInFlight = true;
      const settle = () => {
        listInFlight = false;
        if (listPending) {
          listPending = false;
          get().listSessions();
        }
      };
      set({ sessionsLoading: true, sessionListError: null });
      useConnectionStore
        .getState()
        .sendRpc<SessionList>("session/list")
        .then((data) => {
          const deleted = get().deletedSessionIds;
          set({
            sessions: sortSessions(data.sessions.filter((s) => !deleted[s.id])),
            folders: sortFolders(data.folders ?? []),
            sessionsLoading: false,
            sessionListError: null,
          });
        })
        .catch((err: unknown) => {
          set({
            sessionsLoading: false,
            sessionListError:
              err instanceof Error ? err.message : "Failed to load sessions",
          });
        })
        .finally(() => {
          noteSessionListSettled();
          settle();
        });
    },

    setPrimary: (sessionId, agentId) => {
      const { pendingSessionOp } = get();
      if (pendingSessionOp !== null) return;
      if (sessionControlsLocked(sessionId)) return;
      // Do not clear modelId — agent and model are independently sticky.
      patch(sessionId, { pendingPrimaryId: agentId });
      useConnectionStore
        .getState()
        .sendRpc("agent/set-primary", {
          agent_id: agentId,
          session_id: sessionId,
        })
        .catch(() => {
          patch(sessionId, { pendingPrimaryId: null });
          useToastStore
            .getState()
            .showToast("Failed to set primary agent", "error");
        });
    },

    setModel: (sessionId, modelId) => {
      if (sessionControlsLocked(sessionId)) return;
      useConnectionStore
        .getState()
        .sendRpc("agent/set-model", {
          model_id: modelId,
          session_id: sessionId,
        })
        .catch(() => {
          useToastStore.getState().showToast("Failed to set model", "error");
        });
    },

    setThinkingTier: (sessionId, tier) => {
      if (sessionControlsLocked(sessionId)) return;
      patch(sessionId, { pendingThinkingTier: tier });
      useConnectionStore
        .getState()
        .sendRpc("agent/set-thinking-tier", {
          thinking_tier: tier,
          session_id: sessionId,
        })
        .catch(() => {
          patch(sessionId, { pendingThinkingTier: null });
          useToastStore
            .getState()
            .showToast("Failed to set thinking tier", "error");
        });
    },

    setContextMode: (sessionId, mode) => {
      if (sessionControlsLocked(sessionId)) return;
      patch(sessionId, { pendingContextMode: mode });
      useConnectionStore
        .getState()
        .sendRpc("agent/set-context-mode", {
          context_mode: mode,
          session_id: sessionId,
        })
        .catch(() => {
          patch(sessionId, { pendingContextMode: null });
          useToastStore
            .getState()
            .showToast("Failed to set context mode", "error");
        });
    },

    renameSession: (id, title) => {
      const previous = get().sessions.find((s) => s.id === id)?.title;
      const normalized = title?.trim() || null;
      set((state) => ({
        sessions: state.sessions.map((s) =>
          s.id === id ? { ...s, title: normalized } : s,
        ),
      }));
      useConnectionStore
        .getState()
        .sendRpc("session/rename", { id, title: normalized })
        .catch(() => {
          set((state) => ({
            sessions: state.sessions.map((s) =>
              s.id === id ? { ...s, title: previous } : s,
            ),
          }));
          useToastStore
            .getState()
            .showToast(i18n.t("session.renameFailed"), "error");
        });
    },

    moveSession: (id, folderId) => {
      const previous = get().sessions.find((s) => s.id === id)?.folder_id;
      set((state) => ({
        sessions: state.sessions.map((s) =>
          s.id === id ? { ...s, folder_id: folderId } : s,
        ),
      }));
      useConnectionStore
        .getState()
        .sendRpc("session/move", { id, folder_id: folderId })
        .catch(() => {
          set((state) => ({
            sessions: state.sessions.map((s) =>
              s.id === id ? { ...s, folder_id: previous } : s,
            ),
          }));
          useToastStore
            .getState()
            .showToast(i18n.t("session.moveFailed"), "error");
        });
    },

    createFolder: (name) => {
      const trimmed = name.trim();
      if (!trimmed) return;
      useConnectionStore
        .getState()
        .sendRpc<{ folder_id: string }>("session/folder/create", { name: trimmed })
        .then((result) => {
          // The server pushes a fresh list before this reply, so this is only a
          // fallback for an older/partial reply shape.
          set((state) => {
            if (
              !result?.folder_id ||
              state.folders.some((folder) => folder.id === result.folder_id)
            ) {
              return {};
            }
            return {
              folders: [
                ...state.folders,
                {
                  id: result.folder_id,
                  name: trimmed,
                  sort_order: state.folders.length,
                },
              ],
            };
          });
        })
        .catch(() => {
          useToastStore
            .getState()
            .showToast(i18n.t("session.createFolderFailed"), "error");
        });
    },

    renameFolder: (id, name) => {
      const trimmed = name.trim();
      if (!trimmed) return;
      const previous = get().folders.find((folder) => folder.id === id)?.name;
      set((state) => ({
        folders: state.folders.map((folder) =>
          folder.id === id ? { ...folder, name: trimmed } : folder,
        ),
      }));
      useConnectionStore
        .getState()
        .sendRpc("session/folder/rename", { id, name: trimmed })
        .catch(() => {
          set((state) => ({
            folders: state.folders.map((folder) =>
              folder.id === id ? { ...folder, name: previous ?? folder.name } : folder,
            ),
          }));
          useToastStore
            .getState()
            .showToast(i18n.t("session.renameFolderFailed"), "error");
        });
    },

    deleteFolder: (id) => {
      const previousFolders = get().folders;
      const previousSessions = get().sessions;
      set((state) => ({
        folders: state.folders.filter((folder) => folder.id !== id),
        sessions: state.sessions.map((session) =>
          session.folder_id === id ? { ...session, folder_id: null } : session,
        ),
      }));
      useConnectionStore
        .getState()
        .sendRpc("session/folder/delete", { id })
        .catch(() => {
          set({ folders: previousFolders, sessions: previousSessions });
          useToastStore
            .getState()
            .showToast(i18n.t("session.deleteFolderFailed"), "error");
        });
    },

    setShowSessionList: (open: boolean) => {
      set({ showSessionList: open });
    },
  };
});

attachSiblingStores({ session: useSessionStore });
