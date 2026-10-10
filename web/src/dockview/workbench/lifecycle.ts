import type { DockviewApi } from "dockview-react";

import { noteLayoutSettled } from "../../lib/centreChat";
import {
  applyStagedPopoutBounds,
  preparePopoutRestore,
  stagePopoutBounds,
} from "../config/popoutRestore";
import { bindTabDrag } from "../drag/bindTabDrag";
import { releaseOrphanRenderOverlays } from "../popout/orphanOverlay";
import { bindPopoutWindows } from "../popout/popoutChrome";
import { recoverDefaultLayout } from "./edges";
import { emitPanelRemoved, onPanelRemoved } from "./events";
import { bindDockview, dockview } from "./host";
import { setLayoutFromJsonPending } from "./restoreGate";
import { bindWorkbenchKeys } from "./keys";
import { noteActiveGroup } from "./placement";
import { bindWindowRegistry, onDidRegisterWindow, registeredDocuments } from "./windows";

/**
 * Layout snapshots. The desktop host keeps one file per local workspace,
 * because a local workbench is served from an ephemeral port and browser
 * storage would miss it next launch. Everywhere else browser storage is stable.
 */
export interface LayoutStore {
  load: () => string | null;
  save: (payload: string) => void;
}

const LAYOUT_STORAGE_KEY = "litecode-dockview-layout-v2";

/** Bump when a persisted snapshot can no longer be trusted. Old files are rebuilt. */
export const LAYOUT_SCHEMA_VERSION = 4;

const POPOUT_RESTORE_MS = 5000;

/** Suppress layout saves through popout restore as well. */
let suppressSave = false;
let started: DockviewApi | null = null;
let stopShell: (() => void) | undefined;
let stopListeners: (() => void) | undefined;
let stopLayoutRepair: (() => void) | undefined;

function startWindowShell(api: DockviewApi): () => void {
  const stopRegistry = bindWindowRegistry(api);
  const stopChrome = bindPopoutWindows();
  const stopKeys = bindWorkbenchKeys();
  const stopDrag = bindTabDrag(api);
  const moveSub = api.onDidMovePanel(() => {
    releaseOrphanRenderOverlays(registeredDocuments());
  });
  return () => {
    stopRegistry();
    stopChrome();
    stopKeys();
    stopDrag();
    moveSub.dispose();
  };
}

export function layoutStore(): LayoutStore {
  const host = window.litecode;
  if (
    typeof host?.loadLayout === "function" &&
    typeof host?.saveLayout === "function" &&
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


/**
 * Own layout restore, popout reopening, and layout saves.
 * Safe to call once per Dockview instance.
 */
export function stopWorkbench(): void {
  stopShell?.();
  stopShell = undefined;
  stopListeners?.();
  stopListeners = undefined;
  stopLayoutRepair?.();
  stopLayoutRepair = undefined;
  setLayoutFromJsonPending(false);
  suppressSave = false;
  started = null;
  bindDockview(null);
}

export function startWorkbench(api: DockviewApi): void {
  bindDockview(api);
  if (started === api) return;
  stopWorkbench();
  bindDockview(api);
  started = api;
  stopShell = startWindowShell(api);
  stopLayoutRepair = onPanelRemoved((event) => {
    if (event.component !== "agent") return;
    const live = dockview();
    if (live) queueMicrotask(() => recoverDefaultLayout(live));
  });

  const removeSub = api.onDidRemovePanel((panel) => {
    emitPanelRemoved({ id: panel.id, component: panel.api.component });
  });
  const activeSubs: { dispose(): void }[] = [];
  if (typeof api.onDidActiveGroupChange === "function") {
    activeSubs.push(
      api.onDidActiveGroupChange((group) => {
        noteActiveGroup(group as unknown as Parameters<typeof noteActiveGroup>[0]);
      }),
    );
  }
  noteActiveGroup(api.activeGroup as unknown as Parameters<typeof noteActiveGroup>[0]);

  const store = layoutStore();
  const saved = store.load();
  if (saved) {
    try {
      const parsed = JSON.parse(saved) as { schemaVersion?: number; layout?: unknown };
      if (!parsed || parsed.schemaVersion !== LAYOUT_SCHEMA_VERSION) {
        recoverDefaultLayout(api);
        noteLayoutSettled();
      } else {
        const prepared = preparePopoutRestore(parsed.layout, {
          x: window.screenX,
          y: window.screenY,
        });
        const data = prepared.layout;
        stagePopoutBounds(prepared.bounds);
        const restoredGroups = (data as { popoutGroups?: unknown }).popoutGroups;
        const pendingPopouts = Array.isArray(restoredGroups) ? restoredGroups.length : 0;
        setLayoutFromJsonPending(true);
        suppressSave = true;
        const finishRestore = () => {
          recoverDefaultLayout(api);
          noteLayoutSettled();
        };
        let waiting = pendingPopouts;
        const popoutWatch: { dispose(): void }[] = [];
        let popoutTimer: ReturnType<typeof setTimeout> | undefined;
        const releasePopouts = () => {
          if (popoutTimer !== undefined) clearTimeout(popoutTimer);
          popoutTimer = undefined;
          for (const sub of popoutWatch.splice(0)) sub.dispose();
          stagePopoutBounds(new Map());
          suppressSave = false;
          setLayoutFromJsonPending(false);
        };
        const markSettled = () => {
          waiting -= 1;
          if (waiting <= 0) releasePopouts();
        };
        if (waiting > 0) {
          popoutWatch.push(
            {
              dispose: onDidRegisterWindow((entry) => {
                if (!entry.dockId) return;
                if (applyStagedPopoutBounds(entry.window, entry.dockId)) markSettled();
              }),
            },
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
          // fromJSON applied — business may open panels; keep save suppressed
          // until popouts settle.
          setLayoutFromJsonPending(false);
          try {
            finishRestore();
          } catch {
            recoverDefaultLayout(api);
            noteLayoutSettled();
          }
          if (waiting <= 0) suppressSave = false;
        });
        api.fromJSON(data as never);
        safetyTimer = setTimeout(() => {
          if (!suppressSave) return;
          disposable.dispose();
          setLayoutFromJsonPending(false);
          try {
            finishRestore();
          } catch {
            recoverDefaultLayout(api);
            noteLayoutSettled();
          }
          if (waiting <= 0) suppressSave = false;
        }, 2000);
      }
    } catch {
      setLayoutFromJsonPending(false);
      suppressSave = false;
      recoverDefaultLayout(api);
      noteLayoutSettled();
    }
  } else {
    recoverDefaultLayout(api);
    noteLayoutSettled();
  }

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  const layoutSub = api.onDidLayoutChange(() => {
    if (suppressSave) return;
    if (saveTimer !== undefined) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      store.save(
        JSON.stringify({
          schemaVersion: LAYOUT_SCHEMA_VERSION,
          layout: api.toJSON(),
        }),
      );
    }, 500);
  });
  stopListeners = () => {
    removeSub.dispose();
    for (const sub of activeSubs) sub.dispose();
    layoutSub.dispose();
    if (saveTimer !== undefined) clearTimeout(saveTimer);
  };
}
