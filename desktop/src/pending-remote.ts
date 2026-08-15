import crypto from "node:crypto";

import type { SshSession } from "./ssh-session";

/** In-flight Open remote wizard after auth+deploy, before attach. */
export type PendingRemoteSession = {
  id: string;
  label: string;
  host: string;
  user?: string;
  port?: number;
  credentialKind: "password" | "private_key" | "agent";
  /** Absolute path to materialized key file when credentialKind is private_key. */
  identityFile?: string;
  home: string;
  session: SshSession;
  keyDirectory?: string;
  knownHostsFile: string;
  /** Ephemeral secret kept only until history persist. */
  credentialMaterial?: string;
  /** Set after completeRemoteSession starts serve+tunnel. */
  ready?: {
    token: string;
    baseUrl: string;
    workspace: string;
  };
};

const pending = new Map<string, PendingRemoteSession>();

export function createPendingId(): string {
  return crypto.randomUUID();
}

export function setPendingRemote(session: PendingRemoteSession): void {
  pending.set(session.id, session);
}

export function getPendingRemote(id: string): PendingRemoteSession {
  const session = pending.get(id);
  if (!session) throw new Error("Remote session expired. Start Open remote again.");
  return session;
}

export function takePendingRemote(id: string): PendingRemoteSession {
  const session = getPendingRemote(id);
  pending.delete(id);
  return session;
}

export function deletePendingRemote(id: string): PendingRemoteSession | null {
  const session = pending.get(id) ?? null;
  if (session) pending.delete(id);
  return session;
}

export function clearAllPendingRemotes(): PendingRemoteSession[] {
  const all = [...pending.values()];
  pending.clear();
  return all;
}
