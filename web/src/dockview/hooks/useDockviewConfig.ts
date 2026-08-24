import { useCallback, useRef } from "react";
import type { DockviewApi, DockviewWillDropEvent } from "dockview-react";

import { buildDefaultLayout, ensureDefaultPanels } from "../config/layout";
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
      }
    });

    const saved = localStorage.getItem(LAYOUT_STORAGE_KEY);
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        // Discard layouts from an incompatible schema version (e.g. the old
        // left-only layout) so the restored three-rail default is rebuilt.
        if (!parsed || parsed.schemaVersion !== LAYOUT_SCHEMA_VERSION) {
          buildDefaultLayout(api);
        } else {
        const data = parsed.layout;
        isRestoring = true;
        api.fromJSON(data);
        const disposable = api.onDidLayoutFromJSON(() => {
          isRestoring = false;
          clearTimeout(safetyTimer);
          disposable.dispose();
          const gridGroups = api.groups.filter(
            (g) => g.api.location.type === "grid"
          );
          if (gridGroups.length === 0) {
            buildDefaultLayout(api);
          }
          // fromJSON replaces the whole layout; re-ensure every default panel
          // (Explorer / Search / Source Control / Sessions / Terminal) is
          // present so a version update that adds a new panel never leaves a
          // side panel missing from a restored snapshot.
          ensureDefaultPanels(api);

          // Restore editor tabs for persisted editor panels
          void restoreEditorTabs(api);
        });
        // Safety net: reset after 2s if onDidLayoutFromJSON never fires.
        const safetyTimer = setTimeout(() => {
          if (isRestoring) {
            isRestoring = false;
            disposable.dispose();
            const gridGroups = api.groups.filter(
              (g) => g.api.location.type === "grid"
            );
            if (gridGroups.length === 0) {
              buildDefaultLayout(api);
            }
            ensureDefaultPanels(api);
            // Restore editor tabs (safety net path)
            void restoreEditorTabs(api);
          }
        }, 2000);
        }
      } catch {
        isRestoring = false;
        buildDefaultLayout(api);
      }
    } else {
      buildDefaultLayout(api);
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
