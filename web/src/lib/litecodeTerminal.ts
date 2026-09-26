/** Human terminal client — TerminalHub over WS (UTF-8 data frames). */

import { create } from "zustand";

import type { WireEnvelope } from "../api/agentWs";
import {
  attachSiblingStores,
  useConnectionStore,
} from "../stores/connectionStore";

type DataHandler = (data: string) => void;
type ExitHandler = (code: number | null) => void;

interface BoundTerminal {
  onData: DataHandler;
  onExit: ExitHandler;
}

/** Per-id handlers. Early output lands in `early` until `bindTerminal`. */
const bound = new Map<string, BoundTerminal>();
const early = new Map<string, string>();

/** Cap on bytes stashed before a terminal instance binds its id. */
const EARLY_LIMIT = 65536;

function remember(id: string, data: string): void {
  const next = (early.get(id) ?? "") + data;
  early.set(id, next.length > EARLY_LIMIT ? next.slice(-EARLY_LIMIT) : next);
}

export function bindTerminal(id: string, handlers: BoundTerminal): () => void {
  bound.set(id, handlers);
  const pending = early.get(id);
  if (pending) {
    early.delete(id);
    handlers.onData(pending);
  }
  return () => {
    if (bound.get(id) === handlers) bound.delete(id);
  };
}

/** Drop a pty id this client will not attach (superseded create). */
export function discardTerminal(id: string): void {
  bound.delete(id);
  early.delete(id);
}

/** Best-effort kill of every live terminal (called on app teardown). */
export function closeAllTerminals(): Promise<void> {
  const ids = new Set<string>([...bound.keys(), ...early.keys()]);
  bound.clear();
  early.clear();
  return Promise.all(
    [...ids].map((id) => terminalClose(id).catch(() => {})),
  ).then(() => {});
}

export function handleTerminalWireEnvelope(env: WireEnvelope): boolean {
  if (!("method" in env) || !env.method) return false;
  const params = env.params as Record<string, unknown> | undefined;
  if (!params) return false;

  if (env.method === "terminal/data") {
    const id = typeof params.id === "string" ? params.id : null;
    const data = typeof params.data === "string" ? params.data : null;
    if (!id || data === null) return true;
    const handlers = bound.get(id);
    if (handlers) handlers.onData(data);
    else remember(id, data);
    return true;
  }

  if (env.method === "terminal/exit") {
    const id = typeof params.id === "string" ? params.id : null;
    if (!id) return true;
    const code = typeof params.code === "number" ? params.code : null;
    const handlers = bound.get(id);
    bound.delete(id);
    early.delete(id);
    handlers?.onExit(code);
    return true;
  }

  return false;
}

attachSiblingStores({
  terminal: handleTerminalWireEnvelope,
  terminalCloseAll: () => {
    void closeAllTerminals();
  },
});

export async function terminalCreate(opts?: {
  cols?: number;
  rows?: number;
  cwd?: string;
}): Promise<string> {
  const result = await useConnectionStore
    .getState()
    .sendRpc<{ id: string }>("terminal/create", {
      cols: opts?.cols ?? 80,
      rows: opts?.rows ?? 24,
      ...(opts?.cwd ? { cwd: opts.cwd } : {}),
    });
  if (!result?.id) throw new Error("terminal/create missing id");
  return result.id;
}

export async function terminalWrite(id: string, data: string): Promise<void> {
  await useConnectionStore.getState().sendRpc("terminal/write", { id, data });
}

export async function terminalResize(
  id: string,
  cols: number,
  rows: number,
): Promise<void> {
  await useConnectionStore.getState().sendRpc("terminal/resize", {
    id,
    cols,
    rows,
  });
}

export async function terminalClose(id: string): Promise<void> {
  await useConnectionStore.getState().sendRpc("terminal/close", { id });
}

export interface TerminalTab {
  key: string;
  cwd?: string;
  title: string;
}

interface TerminalTabsState {
  tabs: TerminalTab[];
  activeKey: string | null;
  open: (cwd?: string) => string;
  close: (key: string) => void;
  activate: (key: string) => void;
}

let keySeq = 0;
let titleSeq = 0;

function titleFor(cwd?: string): string {
  const leaf = cwd?.split(/[/\\]/).filter(Boolean).pop();
  if (leaf) return leaf;
  titleSeq += 1;
  return `Terminal ${titleSeq}`;
}

/** Tabs inside the single bottom terminal panel. The pty id stays on the instance. */
export const useTerminalTabs = create<TerminalTabsState>((set, get) => ({
  tabs: [],
  activeKey: null,
  open: (cwd) => {
    keySeq += 1;
    const key = `t${keySeq}`;
    const tab: TerminalTab = { key, cwd, title: titleFor(cwd) };
    set((state) => ({
      tabs: [...state.tabs, tab],
      activeKey: key,
    }));
    return key;
  },
  close: (key) => {
    const current = get().tabs;
    const idx = current.findIndex((tab) => tab.key === key);
    if (idx < 0) return;
    const tabs = current.filter((tab) => tab.key !== key);
    const activeKey =
      get().activeKey === key
        ? (tabs[Math.min(idx, tabs.length - 1)]?.key ?? null)
        : get().activeKey;
    set({ tabs, activeKey });
  },
  activate: (key) => {
    if (get().tabs.some((tab) => tab.key === key)) set({ activeKey: key });
  },
}));
