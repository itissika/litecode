import { useCallback, useRef } from "react";
import type { DockviewApi, DockviewWillDropEvent } from "dockview-react";

import { clearFoldCardOpen } from "../../components/foldCardState";
import { recoverDefaultLayout } from "../config/layout";
import {
  applyStagedPopoutBounds,
  preparePopoutRestore,
  stagePopoutBounds,
} from "../config/popoutRestore";
import { dockIdFromPopoutUrl } from "../config/popoutUrl";
import { dockIdFromLocation } from "../popout/location";
import { noteLayoutSettled } from "../../lib/centreChat";
import { watchGridGroups } from "../../lib/knowledge/panel";
import { buildTabContextMenuItems } from "../config/tabContextMenu";
import { bindTabDrag, dragSourceLocation } from "../drag/bindTabDrag";
import { rejectsDockTarget } from "../drag/tabDragPolicy";
import { bindPopoutWindows } from "../popout/popoutChrome";
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

const POPOUT_RESTORE_MS = 5000;

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

/** Center grid and popout windows are one dock. Edge rails stay on their own. */
export function preventCrossZoneDrop(event: DockviewWillDropEvent, api: DockviewApi) {
  const source = dragSourceLocation(event.getData(), api);
  if (rejectsDockTarget(source, event.group?.api.location.type)) event.preventDefault();
}

export function useDockviewConfig() {
  const apiRef = useRef<DockviewApi | null>(null);

  const onReady = useCallback((event: { api: DockviewApi }) => {
    apiRef.current = event.api;
    const api = event.api;

    useEditorStore.getState().setDockviewApi(api);
    setDockviewApi(api);
    watchGridGroups(api);

    api.onDidRemovePanel((panel) => {
      if (panel.api.component === "editor" && !closingFlags.closingFromStore) {
        useEditorStore.getState().closeTab(panel.api.id);
      }
      // Moves suppress this event. A browser close is the real removal, so the
      // guest page is destroyed here rather than on React unmount.
      if (panel.api.component === "browser") {
        window.litecode?.browserDestroy?.(panel.api.id);
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
          const prepared = preparePopoutRestore(parsed.layout, {
            x: window.screenX,
            y: window.screenY,
          });
          const data = prepared.layout;
          stagePopoutBounds(prepared.bounds);
          const restoredGroups = (data as { popoutGroups?: unknown }).popoutGroups;
          const pendingPopouts = Array.isArray(restoredGroups) ? restoredGroups.length : 0;
          isRestoring = true;
          const finishRestore = () => {
            recoverDefaultLayout(api);
            // Editor panels own their own reads (including after reconnect).
            // Layout JSON only puts the tabs back.
            noteLayoutSettled(api);
          };
          // Popout windows open after fromJSON. Hold saves until they exist,
          // otherwise the next snapshot would record an empty popout list.
          // The previous file already has their screen rectangles; do not
          // write a new snapshot here. moveTo updates screen coordinates
          // asynchronously, and an early save would replace the good ones.
          let waiting = pendingPopouts;
          const popoutWatch: { dispose(): void }[] = [];
          let popoutTimer: ReturnType<typeof setTimeout> | undefined;
          const releasePopouts = () => {
            if (popoutTimer !== undefined) clearTimeout(popoutTimer);
            popoutTimer = undefined;
            for (const sub of popoutWatch.splice(0)) sub.dispose();
            stagePopoutBounds(new Map());
            if (!isRestoring) return;
            isRestoring = false;
          };
          const markSettled = () => {
            waiting -= 1;
            if (waiting <= 0) releasePopouts();
          };
          if (waiting > 0) {
            popoutWatch.push(
              api.onDidAddPopoutGroup((popout) => {
                const dock =
                  dockIdFromLocation(popout.group.api.location) ??
                  dockIdFromPopoutUrl(popout.window.location.href);
                if (applyStagedPopoutBounds(popout.window, dock)) markSettled();
              }),
              api.onDidOpenPopoutWindowFail(() => {
                markSettled();
              }),
            );
            popoutTimer = setTimeout(releasePopouts, POPOUT_RESTORE_MS);
          }
          let safetyTimer: ReturnType<typeof setTimeout> | undefined;
          const disposable = api.onDidLayoutFromJSON(() => {
            if (safetyTimer !== undefined) clearTimeout(safetyTimer);
            disposable.dispose();
            try {
              finishRestore();
            } catch {
              recoverDefaultLayout(api);
              noteLayoutSettled(api);
            }
            if (waiting <= 0) isRestoring = false;
          });
          api.fromJSON(data);
          // Safety net: reset after 2s if onDidLayoutFromJSON never fires.
          safetyTimer = setTimeout(() => {
            if (!isRestoring) return;
            disposable.dispose();
            try {
              finishRestore();
            } catch {
              recoverDefaultLayout(api);
              noteLayoutSettled(api);
            }
            if (waiting <= 0) isRestoring = false;
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

    bindPopoutWindows(api);
    bindTabDrag(api);

    let saveTimer: ReturnType<typeof setTimeout>;
    api.onDidLayoutChange(() => {
      if (isRestoring) return;
      clearTimeout(saveTimer);
      saveTimer = setTimeout(() => {
        store.save(
          JSON.stringify({
            schemaVersion: LAYOUT_SCHEMA_VERSION,
            layout: api.toJSON(),
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
