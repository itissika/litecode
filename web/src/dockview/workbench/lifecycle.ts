import type { DockviewApi } from "dockview-react";

import { noteLayoutSettled } from "../../lib/centreChat";
import {
  applyStagedPopoutBounds,
  preparePopoutRestore,
  stagePopoutBounds,
} from "../config/popoutRestore";
import { dockIdFromPopoutUrl } from "../config/popoutUrl";
import { bindTabDrag } from "../drag/bindTabDrag";
import { bindPopoutWindows } from "../popout/popoutChrome";
import { dockIdFromLocation } from "../popout/location";
import { recoverDefaultLayout } from "./edges";
import { emitPanelRemoved, onPanelRemoved } from "./events";
import { bindDockview, dockview } from "./host";
import { noteActiveGroup } from "./placement";

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

let restoring = false;
let started: DockviewApi | null = null;

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

let layoutRepairBound = false;

/**
 * Own layout restore, popout reopening, and layout saves.
 * Safe to call once per Dockview instance.
 */
export function startWorkbench(api: DockviewApi): void {
  bindDockview(api);
  if (started === api) return;
  started = api;
  if (!layoutRepairBound) {
    layoutRepairBound = true;
    onPanelRemoved((event) => {
      if (event.component !== "agent") return;
      const live = dockview();
      if (live) queueMicrotask(() => recoverDefaultLayout(live));
    });
  }

  api.onDidRemovePanel((panel) => {
    emitPanelRemoved({ id: panel.id, component: panel.api.component });
  });
  if (typeof api.onDidActiveGroupChange === "function") {
    api.onDidActiveGroupChange((group) => {
      noteActiveGroup(group as unknown as Parameters<typeof noteActiveGroup>[0]);
    });
  }
  noteActiveGroup(api.activeGroup as unknown as Parameters<typeof noteActiveGroup>[0]);

  const store = layoutStore();
  const saved = store.load();
  if (saved) {
    try {
      const parsed = JSON.parse(saved) as { schemaVersion?: number; layout?: unknown };
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
        restoring = true;
        const finishRestore = () => {
          recoverDefaultLayout(api);
          noteLayoutSettled(api);
        };
        let waiting = pendingPopouts;
        const popoutWatch: { dispose(): void }[] = [];
        let popoutTimer: ReturnType<typeof setTimeout> | undefined;
        const releasePopouts = () => {
          if (popoutTimer !== undefined) clearTimeout(popoutTimer);
          popoutTimer = undefined;
          for (const sub of popoutWatch.splice(0)) sub.dispose();
          stagePopoutBounds(new Map());
          if (!restoring) return;
          restoring = false;
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
          if (waiting <= 0) restoring = false;
        });
        api.fromJSON(data as never);
        safetyTimer = setTimeout(() => {
          if (!restoring) return;
          disposable.dispose();
          try {
            finishRestore();
          } catch {
            recoverDefaultLayout(api);
            noteLayoutSettled(api);
          }
          if (waiting <= 0) restoring = false;
        }, 2000);
      }
    } catch {
      restoring = false;
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
    if (restoring) return;
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
}
