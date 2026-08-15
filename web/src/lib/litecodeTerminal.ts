/** Human terminal client — TerminalHub over WS (UTF-8 lossy data frames). */

import type { WireEnvelope } from "../api/agentWs";
import { attachSiblingStores, useConnectionStore } from "../stores/connectionStore";

type DataHandler = (id: string, data: string) => void;
type ExitHandler = (id: string, code: number | null) => void;

const dataHandlers = new Set<DataHandler>();
const exitHandlers = new Set<ExitHandler>();

/** Active terminal ids owned by this client, for teardown cleanup. */
const activeTerminals = new Set<string>();

export function registerTerminal(id: string): void {
  activeTerminals.add(id);
}

export function unregisterTerminal(id: string): void {
  activeTerminals.delete(id);
}

/** Best-effort kill of every live terminal (called on app teardown). */
export function closeAllTerminals(): Promise<void> {
  const ids = Array.from(activeTerminals);
  activeTerminals.clear();
  return Promise.all(ids.map((id) => terminalClose(id).catch(() => {}))).then(
    () => {},
  );
}

export function onTerminalData(handler: DataHandler): () => void {
  dataHandlers.add(handler);
  return () => {
    dataHandlers.delete(handler);
  };
}

export function onTerminalExit(handler: ExitHandler): () => void {
  exitHandlers.add(handler);
  return () => {
    exitHandlers.delete(handler);
  };
}

export function handleTerminalWireEnvelope(env: WireEnvelope): boolean {
  if (!("method" in env) || !env.method) return false;
  const params = env.params as Record<string, unknown> | undefined;
  if (!params) return false;

  if (env.method === "terminal/data") {
    const id = typeof params.id === "string" ? params.id : null;
    const data = typeof params.data === "string" ? params.data : null;
    if (!id || data === null) return true;
    for (const h of dataHandlers) h(id, data);
    return true;
  }

  if (env.method === "terminal/exit") {
    const id = typeof params.id === "string" ? params.id : null;
    if (!id) return true;
    const code = typeof params.code === "number" ? params.code : null;
    for (const h of exitHandlers) h(id, code);
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
  const result = await useConnectionStore.getState().sendRpc<{ id: string }>(
    "terminal/create",
    {
      cols: opts?.cols ?? 80,
      rows: opts?.rows ?? 24,
      ...(opts?.cwd ? { cwd: opts.cwd } : {}),
    },
  );
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
