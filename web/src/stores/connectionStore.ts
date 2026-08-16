import { create, type StoreApi, type UseBoundStore } from "zustand";
import type { DockviewApi } from "dockview-react";

import { AgentWsClient, type WireEnvelope } from "../api/agentWs";
import { getAuthToken } from "../api/auth";
import type {
  ConnectionState,
  WireServerHello,
  WorkspaceChanged,
  SettingsChanged,
  ServerStats,
  LogLine,
  SessionSnapshot,
  BufferLoaded,
  BufferItemNotification,
  BufferCompacted,
  CompactLifecycle,
  SubagentBound,
  SessionList,
  SessionLifecycle,
  OperationResult,
  PermissionRequest,
  TurnStarted,
  TurnEventEnvelope,
  TurnFinished,
  BashJobsNotification,
} from "../api/types";
import type { WorkspaceChangeKind } from "../api/workspace";
import { useBashStore } from "./bashStore";
import { useToastStore } from "./toastStore";

/** Module-level dockview API reference, set by useDockviewConfig. */
let _dockviewApi: DockviewApi | null = null;

export function setDockviewApi(api: DockviewApi | null): void {
  _dockviewApi = api;
}

export function getDockviewApi(): DockviewApi | null {
  return _dockviewApi;
}

interface StoreLike {
  getState: () => any;
}
interface SiblingStores {
  session?: StoreLike;
  turn?: StoreLike;
  message?: StoreLike;
  editor?: StoreLike;
  tree?: StoreLike;
  settings?: StoreLike;
  telemetry?: StoreLike;
  lsp?: (env: WireEnvelope) => boolean;
  terminal?: (env: WireEnvelope) => boolean;
  terminalCloseAll?: () => void;
}

/** Legacy forwarded subagent events — must not mutate the parent session UI. */
export function shouldIgnoreForwardedSubagentEvent(
  method: string,
  params: Record<string, unknown> | undefined,
): boolean {
  if (
    typeof params?.parent_session_id !== "string" ||
    params.parent_session_id.length === 0
  ) {
    return false;
  }
  switch (method) {
    case "agent/turn_started":
    case "agent/turn_event":
    case "agent/turn_finished":
    case "buffer/item":
    case "buffer/reverted":
    case "buffer/compacted":
    case "agent/permission_request":
      return true;
    default:
      return false;
  }
}

const siblingStores: SiblingStores = {};

/** Called by sibling stores during module initialisation. */
export function attachSiblingStores(next: Partial<SiblingStores>): void {
  Object.assign(siblingStores, next);
}

interface ConnectionStore {
  state: ConnectionState;
  project: string;
  /** Stable workspace identity from server/hello. */
  workspaceId: string;
  llmEcosystem: string;
  /** Sessions currently subscribed on the live socket. Cleared on every
      drop so AgentPanels re-subscribe themselves on (re)connect. */
  subscribedSessions: Set<string>;

  init: () => void;
  destroy: () => void;
  sendRpc: <T = unknown>(method: string, params?: Record<string, unknown>) => Promise<T>;
  dispatchEnvelope: (env: WireEnvelope) => void;
  ensureSubscribe: (sid: string) => Promise<void>;
  unsubscribeSession: (sid: string) => void;
}

let wsClient: AgentWsClient | null = null;
const pendingSubscriptions = new Map<string, Promise<void>>();

export const useConnectionStore: UseBoundStore<StoreApi<ConnectionStore>> =
  create<ConnectionStore>((set, get) => {
    return {
      state: "disconnected",
      project: "",
      workspaceId: "",
      llmEcosystem: "",
      subscribedSessions: new Set(),

      init: () => {
        if (wsClient) return;

        const authToken = getAuthToken();

        wsClient = new AgentWsClient({
          authToken,
          onConnectionChange: (connection) => {
            if (connection === "connected") {
              set({ state: connection });
            } else {
              // Any non-connected state means the server has dropped our
              // per-socket subscriptions (a fresh socket starts empty).
              // Clear the local record so each AgentPanel re-subscribes
              // itself on (re)connect via its own connection-state effect.
              set({ state: connection, subscribedSessions: new Set() });
            }
            siblingStores.telemetry?.getState().onConnectionChange(connection);
          },
          onEnvelope: (env) => {
            get().dispatchEnvelope(env);
          },
          onError: (error) => {
            // Explicit WS handshake/transport error surface — never silent
            // (G3: handshake failure must be visible to the user).
            useToastStore.getState().showToast(error, "error");
          },
        });

        set({ state: "connecting" });
        wsClient.connect();
      },

      destroy: () => {
        // Kill every live terminal before tearing down the socket so no backend
        // PTY is orphaned on app-level exit.
        siblingStores.terminalCloseAll?.();
        siblingStores.telemetry?.getState().reset();
        wsClient?.disconnect();
        wsClient = null;
        pendingSubscriptions.clear();
        set({
          state: "disconnected",
          project: "",
          workspaceId: "",
          llmEcosystem: "",
          subscribedSessions: new Set(),
        });
      },

      sendRpc: <T = unknown>(
        method: string,
        params?: Record<string, unknown>,
      ): Promise<T> => {
        if (!wsClient || !wsClient.isConnected()) {
          return Promise.reject(
            new Error("No connected WS for RPC: " + method),
          );
        }
        return wsClient.sendJsonRpc<T>(method, params);
      },

      dispatchEnvelope: (env) => {
        // LSP / terminal push frames are handled by sibling helpers first.
        if (siblingStores.lsp?.(env)) {
          return;
        }
        if (siblingStores.terminal?.(env)) {
          return;
        }

        const method: string | undefined = env.method;
        const params: Record<string, unknown> | undefined = env.params;

        if (!method) {
          return;
        }

        // Legacy forwarded-subagent defense: backend no longer emits parent_session_id
        // on parent-session turn/buffer events, but ignore if a stale peer still does.
        if (shouldIgnoreForwardedSubagentEvent(method, params)) {
          return;
        }

        const session = siblingStores.session?.getState();
        const turn = siblingStores.turn?.getState();
        const message = siblingStores.message?.getState();
        const editor = siblingStores.editor?.getState();
        const tree = siblingStores.tree?.getState();
        const settings = siblingStores.settings?.getState();
        const telemetry = siblingStores.telemetry?.getState();

        switch (method) {
          case "server/hello": {
            const hello = params as unknown as WireServerHello;
            if (!hello.workspace_id || !hello.project) {
              console.error(
                "[connection] server/hello missing workspace identity",
                hello,
              );
            }
            settings?.setRevision(hello.settings_revision);
            void settings?.ensureCatalogLoaded();
            void settings?.notifySetupIfNeeded();
            // hello.session_id is empty string in single-WS mode; only set
            // global info (project, workspace_id, models, settings_revision, etc.).
            set({
              project: hello.project,
              workspaceId: hello.workspace_id ?? "",
              llmEcosystem: hello.llm_ecosystem ?? "openai",
            });
            session?.onHello(hello);
            if (hello.project) {
              void window.litecode?.notifyWorkspace?.(hello.project);
            }
            return;
          }

          case "session/snapshot": {
            const snap = params as unknown as SessionSnapshot;
            session?.applySnapshot(snap);
            return;
          }

          case "session/list": {
            session?.onSessionList((params as unknown as SessionList).sessions);
            return;
          }

          case "session/lifecycle": {
            const lc = params as unknown as SessionLifecycle;
            session?.onSessionLifecycle(lc);
            if (lc.event === "turn_started" && lc.turn?.turn_id) {
              turn?.onTurnStarted({
                session_id: lc.session_id,
                turn_id: lc.turn.turn_id,
                input: "",
                step_max: lc.turn.step_max,
              });
            }
            return;
          }

          case "session/attached": {
            const attached = params as { session_id: string; turn: any };
            session?.onSessionAttached(attached);
            return;
          }

          case "buffer/loaded": {
            const loaded = params as unknown as BufferLoaded;
            message?.onBufferLoaded(loaded.session_id, loaded);
            return;
          }

          case "buffer/item": {
            const bi = params as unknown as BufferItemNotification;
            // Stream deltas are rAF-coalesced; seal is immediate. Flush first so
            // pending tokens are not appended onto the sealed full text.
            // `turn` is already getState() — actions live on the slice itself.
            turn?.flushPendingStream?.(bi.session_id);
            message?.onBufferItem(bi.session_id, bi);
            return;
          }

          case "agent/subagent_bound": {
            const bound = params as unknown as SubagentBound;
            message?.onSubagentBound(bound.session_id, bound);
            return;
          }

          case "bash/jobs": {
            const bash = params as unknown as BashJobsNotification;
            if (bash.session_id) {
              useBashStore.getState().applySnapshot(bash.session_id, {
                jobs: bash.jobs ?? [],
                waits: bash.waits ?? [],
              });
            }
            return;
          }

          case "buffer/reverted": {
            const rev = params as unknown as { session_id: string; committed_end: number };
            turn?.clearPendingStream?.(rev.session_id);
            const turnSlice = turn?.byId?.get(rev.session_id);
            if (
              turnSlice?.runState === "running" ||
              turnSlice?.runState === "cancelling"
            ) {
              turn?.onTranscriptReverted?.(rev.session_id);
            }
            message?.onBufferReverted(rev.session_id, rev);
            return;
          }

          case "buffer/compacted": {
            const compacted = params as unknown as BufferCompacted;
            message?.onBufferCompacted(compacted.session_id, compacted);
            return;
          }

          case "session/compact_lifecycle": {
            const life = params as unknown as CompactLifecycle;
            turn?.onCompactLifecycle?.(life);
            session?.applySnapshot(life.snapshot);
            return;
          }

          case "session/compact_started": {
            const started = params as unknown as CompactLifecycle;
            turn?.onCompactLifecycle?.({
              ...started,
              trigger: started.trigger ?? "manual",
              stage: "started",
            });
            session?.applySnapshot(started.snapshot);
            return;
          }

          case "agent/operation_result": {
            session?.onOperationResult(params as unknown as OperationResult);
            return;
          }

          case "agent/permission_request": {
            const pr = params as unknown as PermissionRequest;
            turn?.onPermissionRequest(pr.session_id, pr);
            return;
          }

          case "agent/turn_started": {
            turn?.onTurnStarted(params as unknown as TurnStarted);
            return;
          }

          case "agent/turn_event": {
            turn?.onTurnEvent(params as unknown as TurnEventEnvelope);
            return;
          }

          case "agent/turn_finished": {
            turn?.onTurnFinished(params as unknown as TurnFinished);
            return;
          }

          case "workspace/changed": {
            const { paths, kind } = params as unknown as WorkspaceChanged;
            const changeKind = kind as WorkspaceChangeKind;
            void editor?.handleWorkspaceChange(paths, changeKind);
            void tree?.handleWorkspaceChange(paths, changeKind);
            return;
          }

          case "settings/changed": {
            settings?.onRemoteSettingsChanged(
              params as unknown as SettingsChanged,
            );
            return;
          }

          case "server/stats": {
            telemetry?.onServerStats(params as unknown as ServerStats);
            return;
          }

          case "log/line": {
            telemetry?.onLogLine(params as unknown as LogLine);
            return;
          }
        }
      },

      ensureSubscribe: (sid) => {
        if (!sid) return Promise.reject(new Error("Session ID is required"));
        if (get().subscribedSessions.has(sid)) return Promise.resolve();
        const pending = pendingSubscriptions.get(sid);
        if (pending) return pending;

        let request!: Promise<void>;
        request = get()
          .sendRpc("session/subscribe", { session_id: sid })
          .then(() => {
            if (pendingSubscriptions.get(sid) !== request) return;
            const sessions = new Set(get().subscribedSessions);
            sessions.add(sid);
            set({ subscribedSessions: sessions });
          })
          .finally(() => {
            if (pendingSubscriptions.get(sid) === request) {
              pendingSubscriptions.delete(sid);
            }
          });
        pendingSubscriptions.set(sid, request);
        return request;
      },

      unsubscribeSession: (sid) => {
        pendingSubscriptions.delete(sid);
        const sessions = new Set(get().subscribedSessions);
        if (!sessions.has(sid)) return;
        sessions.delete(sid);
        set({ subscribedSessions: sessions });
        void get().sendRpc("session/unsubscribe", { session_id: sid }).catch(() => {});
      },
    };
  });
