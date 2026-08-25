import { create } from "zustand";
import type {
  WireServerHello,
  SessionSnapshot,
  SessionInfo,
  OperationResult,
  OperationKind,
  PrimaryAgentInfo,
  ModelInfo,
  BufferLoaded,
  TurnStepKind,
  ThinkingTier,
  ContextMode,
} from "../api/types";
import { getModels, type ModelDefinition } from "../api/settings";
import { useConnectionStore, attachSiblingStores, getDockviewApi } from "./connectionStore";
import { useBashStore } from "./bashStore";
import { useToastStore } from "./toastStore";
import { useTurnStore } from "./turnStore";
import { useMessageStore } from "./messageStore";

function definitionsToModelInfo(models: Record<string, ModelDefinition>): ModelInfo[] {
  return Object.entries(models)
    .map(([id, m]) => ({
      id,
      api_model_id: m.config.api_model_id,
      label: m.label,
      context_window: m.config.context_window,
      adapter_id: m.adapter_id,
    }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

/**
 * Most-recently-updated first (event order). The backend returns the list
 * pre-sorted, but live lifecycle events mutate it in place; re-sort so any
 * session with new activity always bubbles to the top. Stable sort — sessions
 * sharing an `updated_at` keep their relative order.
 */
function sortSessions(sessions: SessionInfo[]): SessionInfo[] {
  return [...sessions].sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
}

function sessionControlsLocked(sessionId: string): boolean {
  const slice = useTurnStore.getState().byId.get(sessionId);
  const run = slice?.runState;
  return run === "running" || run === "cancelling" || slice?.compacting === true;
}

export interface SessionSlice {
  /** Sticky primary agent id — hydrated from snapshot.agent_id. */
  activePrimary: string;
  pendingPrimaryId: string | null;
  /** Sticky catalog config id — hydrated from snapshot.model_id; null = unset. */
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
  /** Highest user-detail k with a nonempty file patch; null = hide Revert files. */
  maxFileRevertK: number | null;
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
    maxFileRevertK: null,
  };
}

function getSlice(byId: Map<string, SessionSlice>, sessionId: string): SessionSlice {
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
  sessionsLoading: boolean;
  sessionListError: string | null;
  pendingSessionOp: OperationKind | null;
  statusMessage: string | null;
  showSessionList: boolean;
  availableModels: ModelInfo[];
}

interface SessionStore extends SessionState {
  // Server notification handlers
  onHello: (hello: WireServerHello) => void;
  applySnapshot: (snap: SessionSnapshot) => void;
  onSessionList: (sessions: SessionInfo[]) => void;
  onSessionLifecycle: (params: {
    session_id: string;
    event:
      | "deleted"
      | "turn_started"
      | "turn_updated"
      | "turn_finished"
      | "preview_updated"
      | "turn_step";
    turn: any;
    preview?: string;
    updated_at?: number;
    step_kind?: TurnStepKind;
  }) => void;
  onSessionAttached: (params: { session_id: string; turn: any }) => void;
  onOperationResult: (op: OperationResult) => void;

  // RPC methods
  newSession: () => void;
  deleteSession: (id: string) => void;
  listSessions: () => void;
  setPrimary: (sessionId: string, agentId: string) => void;
  setModel: (sessionId: string, modelId: string) => void;
  setThinkingTier: (sessionId: string, tier: ThinkingTier) => void;
  setContextMode: (sessionId: string, mode: ContextMode) => void;
  refreshAvailableModels: () => Promise<void>;

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
      maxFileRevertK:
        snap.max_file_revert_k === undefined || snap.max_file_revert_k === null
          ? null
          : snap.max_file_revert_k,
    });

    // Snapshot turn is hydrate-only. `turn: null` (compact / idle session) must
    // not force idle — that belongs to turn_finished. A compact snapshot that
    // lands after the next turn has started would otherwise drop stream deltas.
    if (snap.turn) {
      useTurnStore.getState().applySnapshotTurn(sessionId, snap.turn);
    }
    useTurnStore.getState().applySnapshotMeter(sessionId, snap);
    if (snap.bash) {
      useBashStore.getState().applySnapshot(sessionId, snap.bash);
    }

    if (snap.buffer.next_seq === 0) {
      useTurnStore.getState().clearPendingStream(sessionId);
      set({ pendingSessionOp: null });
      return;
    }

    // Cold-start history only. Growth after the window exists is buffer/item.
    const msgSlice = useMessageStore.getState().bySession.get(sessionId);
    const needsInitialLoad =
      !msgSlice ||
      (msgSlice.toSeq === 0 && msgSlice.messages.length === 0);

    if (needsInitialLoad) {
      useTurnStore.getState().clearPendingStream(sessionId);
      const toSeq = snap.buffer.next_seq;
      const fromSeq = Math.max(0, toSeq - 40);
      useConnectionStore.getState().sendRpc<BufferLoaded>("buffer/load", { from_seq: fromSeq, to_seq: toSeq, session_id: sessionId })
        .then((loaded) => {
          useMessageStore.getState().onBufferLoaded(sessionId, loaded);
        })
        .catch(() => {
          set({ statusMessage: "Failed to load transcript buffer" });
        });
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
        const local = useMessageStore.getState().bySession.get(op.snapshot.session_id);
        if (local && local.toSeq > op.snapshot.buffer.next_seq) {
          useTurnStore.getState().clearPendingStream(op.snapshot.session_id);
          useTurnStore.getState().onTranscriptReverted(op.snapshot.session_id);
          useMessageStore.getState().onBufferReverted(op.snapshot.session_id, {
            session_id: op.snapshot.session_id,
            last_seq: op.snapshot.buffer.last_seq,
            next_seq: op.snapshot.buffer.next_seq,
          });
        }
        useToastStore
          .getState()
          .showToast("Transcript reverted", "success");
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

    const errMsg =
      op.error?.message ?? op.error?.code ?? "Operation failed";

    if (op.op === "new_session") {
      useToastStore.getState().showToast(errMsg, "error");
      set({
        pendingSessionOp: null,
        statusMessage: errMsg,
      });
      return;
    }

    if (op.op === "delete_session") {
      // RPC catch already toasts; refresh list so optimistic remove can roll back.
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
    sessionsLoading: false,
    sessionListError: null,
    pendingSessionOp: null,
    statusMessage: null,
    showSessionList: false,
    availableModels: [],

    onHello: (hello: WireServerHello) => {
      set((s) => ({
        activePrimary: hello.active_primary ?? s.activePrimary,
        primaryAgents: hello.primary_agents ?? s.primaryAgents,
        pendingPrimaryId: null,
        availableModels: hello.models ?? [],
        project: hello.project || s.project,
      }));
      // REST catalog is authoritative after Settings edits; hello may be stale
      // until the next reconnect — refresh so the switcher never goes empty.
      void get().refreshAvailableModels();
    },

    applySnapshot,

    onSessionList: (sessions: SessionInfo[]) => {
      set({ sessions: sortSessions(sessions), sessionsLoading: false, sessionListError: null });
    },

    onSessionLifecycle: (params) => {
      const { session_id, event, turn, preview, updated_at, step_kind } = params;
      set((state) => {
        const exists = state.sessions.some((s) => s.id === session_id);
        if (event === "deleted") {
          useConnectionStore.getState().unsubscribeSession(session_id);
          getDockviewApi()?.getPanel(`agent-${session_id}`)?.api.close();
          return exists
            ? { sessions: sortSessions(state.sessions.filter((s) => s.id !== session_id)) }
            : {};
        }
        // turn_started / turn_updated / turn_finished may arrive for a session
        // that is not yet in the local list (e.g. a push arrived before the
        // initial pull completed). Upsert it so the live status is not dropped.
        if (!exists) {
          if (event === "turn_finished") {
            useTurnStore.getState().onLifecycleTurnFinished(
              session_id,
              turn?.turn_id,
            );
          }
          const fresh: SessionInfo = {
            id: session_id,
            project: "",
            updated_at: updated_at ?? Date.now(),
            preview: preview ?? "",
            running: event !== "turn_finished",
            turn: turn ?? null,
            agent_id: "",
            model_id: null,
            api_model_id: "",
            step_kinds: step_kind ? [step_kind] : [],
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
                    step_kinds:
                      step_kind
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
                ? { ...s, running: true, turn: turn ?? s.turn, step_kinds: [] }
                : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          case "turn_updated": {
            const sessions = state.sessions.map((s) =>
              s.id === session_id ? { ...s, running: true, turn: turn ?? s.turn } : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          case "turn_finished": {
            // Keep `step_kinds` for the recap; the UI holds it for a while then
            // transitions back to idle on its own timer. Cleared on next start.
            useTurnStore.getState().onLifecycleTurnFinished(
              session_id,
              turn?.turn_id,
            );
            const sessions = state.sessions.map((s) =>
              s.id === session_id ? { ...s, running: false, turn: null } : s,
            );
            return { sessions: sortSessions(sessions) };
          }
          default:
            return {};
        }
      });
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
      useConnectionStore.getState().sendRpc<{ session_id: string }>("session/new").then((result) => {
        set({ pendingSessionOp: null, statusMessage: null });
        // Open agent panel for the new session via dockview API.
        const api = getDockviewApi();
        if (api && result.session_id) {
          const sid = result.session_id;
          const panelId = `agent-${sid}`;
          if (!api.getPanel(panelId)) {
            // Open in the center grid, not the active edge group (e.g. Sessions).
            const gridGroups = api.groups.filter((g) => g.api.location.type === "grid");
            let position: { referenceGroup: string } | undefined;
            if (gridGroups.length === 0) {
              const group = api.addGroup();
              position = { referenceGroup: group.id };
            } else {
              position = { referenceGroup: gridGroups[0].api.id };
            }
            api.addPanel({
              id: panelId,
              component: "agent",
              title: sid.slice(0, 8),
              params: { sessionId: sid },
              tabComponent: "agent",
              position,
            });
          }
        }
      }).catch(() => {
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
      if (target?.running) return;

      // Optimistic: gone from list and panel immediately.
      set({ sessions: sortSessions(get().sessions.filter((s) => s.id !== id)) });
      useConnectionStore.getState().unsubscribeSession(id);
      getDockviewApi()?.getPanel(`agent-${id}`)?.api.close();

      useConnectionStore
        .getState()
        .sendRpc("session/delete", { id })
        .catch(() => {
          useToastStore.getState().showToast("Failed to delete session", "error");
          get().listSessions();
        });
    },

    listSessions: () => {
      set({ sessionsLoading: true, sessionListError: null });
      useConnectionStore.getState().sendRpc<{ sessions: SessionInfo[] }>("session/list").then((data) => {
        set({ sessions: sortSessions(data.sessions), sessionsLoading: false, sessionListError: null });
      }).catch((err: unknown) => {
        set({
          sessionsLoading: false,
          sessionListError: err instanceof Error ? err.message : "Failed to load sessions",
        });
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
        .sendRpc("agent/set-primary", { agent_id: agentId, session_id: sessionId })
        .catch(() => {
          patch(sessionId, { pendingPrimaryId: null });
          useToastStore.getState().showToast("Failed to set primary agent", "error");
        });
    },

    setModel: (sessionId, modelId) => {
      if (sessionControlsLocked(sessionId)) return;
      useConnectionStore
        .getState()
        .sendRpc("agent/set-model", { model_id: modelId, session_id: sessionId })
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
          useToastStore.getState().showToast("Failed to set thinking tier", "error");
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
          useToastStore.getState().showToast("Failed to set context mode", "error");
        });
    },

    refreshAvailableModels: async () => {
      try {
        const models = await getModels();
        const availableModels = definitionsToModelInfo(models);
        const valid = new Set(Object.keys(models));
        const byId = new Map(get().byId);
        for (const [sid, slice] of byId) {
          if (slice.modelId && !valid.has(slice.modelId)) {
            byId.set(sid, {
              ...slice,
              modelId: null,
              apiModelId: "",
              label: "",
            });
          }
        }
        set({ availableModels, byId });
      } catch {
        /* keep existing list */
      }
    },

    setShowSessionList: (open: boolean) => {
      set({ showSessionList: open });
    },
  };
});

attachSiblingStores({ session: useSessionStore });
