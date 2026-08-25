import { useCallback, useRef } from "react";
import type { DockviewApi, DockviewWillDropEvent } from "dockview-react";

import { recoverDefaultLayout } from "../config/layout";
import { buildTabContextMenuItems } from "../config/tabContextMenu";
import { closingFlags } from "../config/sharedFlags";
import { useEditorStore } from "../../stores/editorStore";
import { useConnectionStore, setDockviewApi } from "../../stores/connectionStore";
import { readFile } from "../../api/workspace";
import { languageFromPath } from "../../utils/language";

const LAYOUT_STORAGE_KEY = "litecode-dockview-layout-v2";
// Bump when the default layout shape changes so incompatible persisted
// snapshots (e.g. the old left-only layout) are discarded and rebuilt.
const LAYOUT_SCHEMA_VERSION = 3;
let isRestoring = false;

/** Restore editor tabs for every persisted editor panel (single shared
 *  implementation used by both the layout-restore callback and its 2s safety
 *  net — FE-08 dedup). */
async function restoreEditorTabs(api: DockviewApi): Promise<void> {
  const editorPanels = api.panels.filter(
    (p) => p.api.component === "editor",
  );
  for (const panel of editorPanels) {
    const path = panel.api.id;
    const store = useEditorStore.getState();
    if (store.tabs.find((t) => t.path === path)) continue;
    try {
      const content = await readFile(path);
      useEditorStore.setState((s) => ({
        tabs: [...s.tabs, {
          path,
          content,
          savedContent: content,
          dirty: false,
          language: languageFromPath(path),
          loading: false,
          error: null,
        }],
      }));
    } catch {
      // file may not exist — skip
    }
  }
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
      // When an agent panel is closed, unsubscribe from that session.
      // No confirmation dialog, no cancel turn — just unsubscribe.
      // Panel id follows convention "agent-${sessionId}".
      if (panel.api.component === "agent" && panel.api.id?.startsWith("agent-")) {
        const sid = panel.api.id.slice("agent-".length);
        if (sid) {
          useConnectionStore.getState().unsubscribeSession(sid);
        }
        // Closing a stale agent tab (session gone) can empty a group or leave
        // a restored edge rail blank. Re-ensure the default chrome after the
        // removal settles — no-op when the rails are already healthy.
        queueMicrotask(() => recoverDefaultLayout(api));
      }
    });

    const saved = localStorage.getItem(LAYOUT_STORAGE_KEY);
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        // Discard layouts from an incompatible schema version (e.g. the old
        // left-only layout) so the restored three-rail default is rebuilt.
        if (!parsed || parsed.schemaVersion !== LAYOUT_SCHEMA_VERSION) {
          recoverDefaultLayout(api);
        } else {
        const data = parsed.layout;
        isRestoring = true;
        const finishRestore = () => {
          recoverDefaultLayout(api);
          void restoreEditorTabs(api);
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
      }
    } else {
      recoverDefaultLayout(api);
    }

    let saveTimer: ReturnType<typeof setTimeout>;
    api.onDidLayoutChange(() => {
      if (isRestoring) return;
      clearTimeout(saveTimer);
      saveTimer = setTimeout(() => {
        const data = api.toJSON();
        localStorage.setItem(
          LAYOUT_STORAGE_KEY,
          JSON.stringify({ schemaVersion: LAYOUT_SCHEMA_VERSION, layout: data }),
        );
      }, 500);
    });

    api.onUnhandledDragOver((e) => {
      e.accept();
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
