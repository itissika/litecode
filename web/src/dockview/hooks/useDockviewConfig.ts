import { useCallback, useRef } from "react";
import type { DockviewApi, DockviewWillDropEvent } from "dockview-react";

import { clearFoldCardOpen } from "../../components/foldCardState";
import { recoverDefaultLayout } from "../config/layout";
import { noteLayoutSettled } from "../../lib/centreChat";
import { buildTabContextMenuItems } from "../config/tabContextMenu";
import { closingFlags } from "../config/sharedFlags";
import { useEditorStore } from "../../stores/editorStore";
import {
  useConnectionStore,
  setDockviewApi,
} from "../../stores/connectionStore";

/** Session id encoded in an agent or subagent panel id, or null. */
function sessionIdFromPanel(
  component: string | undefined,
  panelId: string | undefined,
): string | null {
  if (!panelId) return null;
  const prefix =
    component === "agent"
      ? "agent-"
      : component === "subagent"
        ? "subagent-"
        : null;
  if (!prefix || !panelId.startsWith(prefix)) return null;
  return panelId.slice(prefix.length) || null;
}

const LAYOUT_STORAGE_KEY = "litecode-dockview-layout-v2";
// Bump when the default layout shape changes so incompatible persisted
// snapshots (e.g. the old left-only layout) are discarded and rebuilt.
const LAYOUT_SCHEMA_VERSION = 3;
let isRestoring = false;

/**
 * Where a snapshot lives. The desktop host keeps one file per local workspace,
 * because a local workbench is served from `http://127.0.0.1:<ephemeral port>`
 * (see desktop/src/sidecar.ts): browser storage is keyed by origin, so its
 * bucket changes every launch and a snapshot saved there is unreachable next
 * boot. Everywhere else browser storage is stable — the dev server, a
 * fixed-port `serve`, a remote workbench — so it stays the store there.
 */
interface LayoutStore {
  load: () => string | null;
  save: (payload: string) => void;
}

/** Exported for tests; the workbench is the only production caller. */
export function layoutStore(): LayoutStore {
  const host = window.litecode;
  if (
    typeof host?.loadLayout === "function" &&
    typeof host.saveLayout === "function" &&
    host.getSessionMode?.() === "local"
  ) {
    return {
      load: () => host.loadLayout?.() ?? null,
      save: (payload) => host.saveLayout?.(payload),
    };
  }
  return {
    load: () => localStorage.getItem(LAYOUT_STORAGE_KEY),
    save: (payload) => {
      localStorage.setItem(LAYOUT_STORAGE_KEY, payload);
    },
  };
}

function preventCrossZoneDrop(event: DockviewWillDropEvent, api: DockviewApi) {
  const panel = event.panel;
  const data = event.getData();

  const sourcePanelId = panel?.api.id ?? data?.panelId;
  if (!sourcePanelId) return;

  const sourcePanel = api.getPanel(sourcePanelId);
  if (!sourcePanel) return;

  const sourceZone = sourcePanel.api.location.type;

  if (!event.group) {
    if (sourceZone === "edge") {
      event.preventDefault();
    }
    return;
  }

  const targetZone = event.group.api.location.type;
  if (sourceZone !== targetZone) {
    event.preventDefault();
  }
}

export function useDockviewConfig() {
  const apiRef = useRef<DockviewApi | null>(null);

  const onReady = useCallback((event: { api: DockviewApi }) => {
    apiRef.current = event.api;
    const api = event.api;

    useEditorStore.getState().setDockviewApi(api);
    setDockviewApi(api);

    api.onDidRemovePanel((panel) => {
      if (panel.api.component === "editor" && !closingFlags.closingFromStore) {
        useEditorStore.getState().closeTab(panel.api.id);
      }
      // Fold open-intent lives in a module map, not in the panel. Drop it
      // here, still inside close(), before React unmounts. The unmount
      // effect runs after paint, so a reopen can mount new FoldCards,
      // read the old keepopen, and write it back.
      const sid = sessionIdFromPanel(panel.api.component, panel.api.id);
      if (sid) clearFoldCardOpen(sid);

      // When an agent panel is closed, unsubscribe from that session.
      // No confirmation dialog, no cancel turn — just unsubscribe.
      // Panel id follows convention "agent-${sessionId}".
      if (
        panel.api.component === "agent" &&
        panel.api.id?.startsWith("agent-")
      ) {
        if (sid) useConnectionStore.getState().unsubscribeSession(sid);
        // Closing a stale agent tab (session gone) can empty a group or leave
        // a restored edge rail blank. Re-ensure the default chrome after the
        // removal settles — no-op when the rails are already healthy.
        queueMicrotask(() => recoverDefaultLayout(api));
      }
    });

    const store = layoutStore();
    const saved = store.load();
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        // Discard layouts from an incompatible schema version (e.g. the old
        // left-only layout) so the restored three-rail default is rebuilt.
        if (!parsed || parsed.schemaVersion !== LAYOUT_SCHEMA_VERSION) {
          recoverDefaultLayout(api);
          noteLayoutSettled(api);
        } else {
          const data = parsed.layout;
          isRestoring = true;
          const finishRestore = () => {
            recoverDefaultLayout(api);
            // Editor panels own their own reads (including after reconnect).
            // Layout JSON only puts the tabs back.
            noteLayoutSettled(api);
          };
          let safetyTimer: ReturnType<typeof setTimeout> | undefined;
          const disposable = api.onDidLayoutFromJSON(() => {
            isRestoring = false;
            if (safetyTimer !== undefined) clearTimeout(safetyTimer);
            disposable.dispose();
            try {
              finishRestore();
            } catch {
              recoverDefaultLayout(api);
              noteLayoutSettled(api);
            }
          });
          api.fromJSON(data);
          // Safety net: reset after 2s if onDidLayoutFromJSON never fires.
          safetyTimer = setTimeout(() => {
            if (isRestoring) {
              isRestoring = false;
              disposable.dispose();
              finishRestore();
            }
          }, 2000);
        }
      } catch {
        isRestoring = false;
        recoverDefaultLayout(api);
        noteLayoutSettled(api);
      }
    } else {
      recoverDefaultLayout(api);
      noteLayoutSettled(api);
    }

    let saveTimer: ReturnType<typeof setTimeout>;
    api.onDidLayoutChange(() => {
      if (isRestoring) return;
      clearTimeout(saveTimer);
      saveTimer = setTimeout(() => {
        const data = api.toJSON();
        store.save(
          JSON.stringify({
            schemaVersion: LAYOUT_SCHEMA_VERSION,
            layout: data,
          }),
        );
      }, 500);
    });

  }, []);

  const onWillDrop = useCallback((event: DockviewWillDropEvent) => {
    if (apiRef.current) {
      preventCrossZoneDrop(event, apiRef.current);
    }
  }, []);

  const getTabContextMenuItems = buildTabContextMenuItems;

  return { apiRef, onReady, onWillDrop, getTabContextMenuItems };
}
