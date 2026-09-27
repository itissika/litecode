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

export interface CreatedTerminal {
  id: string;
  /** Shell program label (`bash` / `powershell` / …), when the server reports it. */
  shell?: string;
}

export async function terminalCreate(opts?: {
  cols?: number;
  rows?: number;
  cwd?: string;
}): Promise<CreatedTerminal> {
  const result = await useConnectionStore
    .getState()
    .sendRpc<CreatedTerminal>("terminal/create", {
      cols: opts?.cols ?? 80,
      rows: opts?.rows ?? 24,
      ...(opts?.cwd ? { cwd: opts.cwd } : {}),
    });
  if (!result?.id) throw new Error("terminal/create missing id");
  return { id: result.id, shell: result.shell };
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

/** Cap on a tracked command line — a row label, not a transcript. */
const COMMAND_LINE_MAX = 200;

/** Index just past the escape sequence starting at `start` (which holds ESC). */
function escapeEnd(data: string, start: number): number {
  let i = start + 1;
  if (i >= data.length) return i;
  const next = data[i]!;
  if (next === "[") {
    i += 1;
    while (i < data.length) {
      const code = data.charCodeAt(i);
      if (code >= 0x40 && code <= 0x7e) return i + 1; // final byte
      i += 1;
    }
    return i;
  }
  if (next === "]") {
    i += 1;
    while (i < data.length) {
      const code = data.charCodeAt(i);
      if (code === 0x07) return i + 1; // BEL
      if (code === 0x1b && data[i + 1] === "\\") return i + 2; // ST
      i += 1;
    }
    return i;
  }
  return i + 1; // SS3 / two-char sequence
}

/**
 * Track the interactive line a keystroke chunk edits.
 *
 * Returns the line left in the buffer plus every command this chunk submitted
 * (a `\r`). Escape sequences — arrows, function keys, bracketed-paste markers —
 * are skipped, so history recall does not corrupt the line; Ctrl-C / Ctrl-U /
 * Ctrl-L clear it; Tab and the remaining control keys are ignored. This is the
 * best a raw pty gives us: the shell never reports what it ran.
 */
export function trackCommandLine(
  line: string,
  data: string,
): { line: string; commands: string[] } {
  let next = line;
  const commands: string[] = [];
  for (let i = 0; i < data.length; i += 1) {
    const ch = data[i]!;
    if (ch === "\x1b") {
      i = escapeEnd(data, i) - 1;
      continue;
    }
    if (ch === "\r" || ch === "\n") {
      const command = next.trim();
      next = "";
      if (command) commands.push(command);
      continue;
    }
    if (ch === "\x7f" || ch === "\b") {
      next = next.slice(0, -1);
      continue;
    }
    if (ch === "\x03" || ch === "\x15" || ch === "\x0c") {
      next = "";
      continue;
    }
    if (ch.charCodeAt(0) < 0x20) continue;
    if (next.length < COMMAND_LINE_MAX) next += ch;
  }
  return { line: next, commands };
}

export interface TerminalTab {
  key: string;
  cwd?: string;
  title: string;
  /** Shell backing this tab (`bash`, `powershell`, …) once the pty exists. */
  shell?: string;
  /** Last command line the user submitted in this tab. */
  lastCommand?: string;
}

interface TerminalTabsState {
  tabs: TerminalTab[];
  activeKey: string | null;
  open: (cwd?: string) => string;
  close: (key: string) => void;
  activate: (key: string) => void;
  noteShell: (key: string, shell: string) => void;
  noteCommand: (key: string, command: string) => void;
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
  noteShell: (key, shell) => {
    if (!shell) return;
    set((state) => ({
      tabs: state.tabs.map((tab) =>
        tab.key === key && tab.shell !== shell ? { ...tab, shell } : tab,
      ),
    }));
  },
  noteCommand: (key, command) => {
    const trimmed = command.trim();
    if (!trimmed) return;
    set((state) => ({
      tabs: state.tabs.map((tab) =>
        tab.key === key && tab.lastCommand !== trimmed
          ? { ...tab, lastCommand: trimmed }
          : tab,
      ),
    }));
  },
}));
