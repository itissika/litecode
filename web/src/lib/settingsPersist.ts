import { useEffect, useRef } from "react";

import { turnMapIsBusy, useTurnStore } from "../stores/turnStore";

export type PersistStatus = "idle" | "pending" | "saving" | "saved" | "invalid" | "error";

export type SerializeResult<P> = { ok: P } | { skip: "unchanged" | "invalid" };

export function isPersistBusy(status: PersistStatus): boolean {
  return status === "pending" || status === "saving";
}

/** Store snapshots must not replace an incomplete local draft (status `invalid`). */
export function shouldHydrateDraftFromStore(status: PersistStatus): boolean {
  return !isPersistBusy(status) && status !== "invalid";
}

export const SETTINGS_PERSIST_ERROR_CHANNEL = "settings-persist-error";

type FlushFn = () => Promise<void>;

const registeredFlushes = new Set<FlushFn>();

export function registerSettingsFlush(flush: FlushFn): () => void {
  registeredFlushes.add(flush);
  return () => {
    registeredFlushes.delete(flush);
  };
}

export async function flushRegisteredSettings(): Promise<void> {
  await Promise.all([...registeredFlushes].map((flush) => flush()));
}

export interface SettingsPersistOptions<D, P> {
  serialize: (draft: D) => SerializeResult<P>;
  commit: (payload: P) => Promise<void>;
  revert: () => void;
  debounceMs: number;
  setStatus: (status: PersistStatus) => void;
  fingerprint?: (payload: P) => string;
}

/** Recursively sort object keys so HashMap-shaped GET echoes match the PUT we just sent. */
function canonicalJson(value: unknown): unknown {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(canonicalJson);
  const obj = value as Record<string, unknown>;
  const sorted: Record<string, unknown> = {};
  for (const key of Object.keys(obj).sort()) {
    const v = obj[key];
    if (v === undefined) continue;
    sorted[key] = canonicalJson(v);
  }
  return sorted;
}

function defaultFingerprint(payload: unknown): string {
  return JSON.stringify(canonicalJson(payload));
}

/**
 * Debounced, coalesced persist controller. Last committed fingerprint skips
 * RPCs; invalid payloads never PUT; failures revert the UI to the snapshot.
 */
export class SettingsPersistController<D, P> {
  private gen = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private inflight: Promise<void> | null = null;
  private latest: D;
  private lastCommitted: string | null;
  private pendingFp: string | null = null;
  private everCommitted = false;
  private disposed = false;
  private blocked = false;
  private savedTimer: ReturnType<typeof setTimeout> | null = null;
  private fingerprint: (payload: P) => string;

  constructor(
    initial: D,
    private readonly opts: SettingsPersistOptions<D, P>,
  ) {
    this.latest = initial;
    this.fingerprint = opts.fingerprint ?? defaultFingerprint;
    const first = opts.serialize(initial);
    this.lastCommitted = "ok" in first ? this.fingerprint(first.ok) : null;
  }

  schedule(draft: D, immediate = false): void {
    if (this.disposed) return;
    this.latest = draft;
    // Blocked (a turn is running — the backend rejects settings writes):
    // hold the draft without arming a doomed PUT. Unblocking re-schedules it.
    if (this.blocked) return;
    const result = this.opts.serialize(draft);
    if ("skip" in result) {
      this.clearTimer();
      this.pendingFp = null;
      if (result.skip === "invalid") {
        this.opts.setStatus("invalid");
      } else if (!this.inflight) {
        this.opts.setStatus(this.everCommitted ? "saved" : "idle");
      }
      return;
    }
    if (this.lastCommitted === this.fingerprint(result.ok)) {
      this.clearTimer();
      this.pendingFp = null;
      if (!this.inflight) this.opts.setStatus(this.everCommitted ? "saved" : "idle");
      return;
    }
    const fp = this.fingerprint(result.ok);
    if (this.pendingFp === fp && (this.timer !== null || this.inflight)) return;
    this.gen += 1;
    this.pendingFp = fp;
    this.opts.setStatus("pending");
    this.clearTimer();
    const delay = immediate ? 0 : this.opts.debounceMs;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.run();
    }, delay);
  }

  /** Cancel the pending debounce while blocked; re-schedule the held draft on release. */
  setBlocked(blocked: boolean): void {
    if (this.blocked === blocked) return;
    this.blocked = blocked;
    if (blocked) {
      this.clearTimer();
    } else {
      this.schedule(this.latest);
    }
  }

  async flush(): Promise<void> {
    if (this.disposed || this.blocked) return;
    this.clearTimer();
    const result = this.opts.serialize(this.latest);
    if ("skip" in result && result.skip === "invalid") {
      this.opts.setStatus("invalid");
      return;
    }
    await this.run();
    if (this.inflight) await this.inflight;
  }

  dispose(): void {
    this.disposed = true;
    this.clearTimer();
    if (this.savedTimer !== null) {
      clearTimeout(this.savedTimer);
      this.savedTimer = null;
    }
  }

  private clearTimer(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }

  private async run(): Promise<void> {
    if (this.disposed) return;
    if (this.inflight) {
      await this.inflight;
      if (this.disposed) return;
    }
    const started = this.gen;
    const draft = this.latest;
    const result = this.opts.serialize(draft);
    if ("skip" in result) {
      if (result.skip === "invalid") this.opts.setStatus("invalid");
      else this.opts.setStatus(this.everCommitted ? "saved" : "idle");
      return;
    }
    if (this.lastCommitted === this.fingerprint(result.ok)) {
      this.opts.setStatus(this.everCommitted ? "saved" : "idle");
      return;
    }
    this.opts.setStatus("saving");
    const payload = result.ok;
    const work = (async () => {
      try {
        await this.opts.commit(payload);
        if (this.disposed) return;
        this.lastCommitted = this.fingerprint(payload);
        this.everCommitted = true;
        this.pendingFp = null;
        if (this.gen === started) {
          this.opts.setStatus("saved");
          this.savedTimer = setTimeout(() => {
            this.savedTimer = null;
            if (this.disposed || this.gen !== started) return;
            this.opts.setStatus("idle");
          }, 800);
        } else {
          void this.run();
        }
      } catch {
        if (this.disposed) return;
        this.opts.revert();
        this.opts.setStatus("error");
      }
    })();
    this.inflight = work;
    try {
      await work;
    } finally {
      if (this.inflight === work) this.inflight = null;
    }
  }
}

export function useSettingsPersist<D, P>(
  draft: D,
  opts: SettingsPersistOptions<D, P> & { enabled?: boolean },
): void {
  // The backend rejects settings writes while any turn is running. Gate the
  // controller on turn-busy centrally: while blocked nothing is scheduled or
  // flushed (no doomed PUT + error/revert); when the turn ends the held draft
  // saves itself. The controller is NOT disposed for this — a fresh one would
  // treat the dirty draft as its committed baseline and swallow the edit.
  const turnBusy = useTurnStore((s) => turnMapIsBusy(s.byId));
  const enabled = opts.enabled ?? true;
  const controllerRef = useRef<SettingsPersistController<D, P> | null>(null);
  const optsRef = useRef(opts);
  optsRef.current = opts;

  useEffect(() => {
    if (!enabled) {
      controllerRef.current?.dispose();
      controllerRef.current = null;
      return;
    }
    const bound: SettingsPersistOptions<D, P> = {
      debounceMs: optsRef.current.debounceMs,
      serialize: (d) => optsRef.current.serialize(d),
      commit: (p) => optsRef.current.commit(p),
      revert: () => optsRef.current.revert(),
      setStatus: (s) => optsRef.current.setStatus(s),
      fingerprint: optsRef.current.fingerprint,
    };
    const controller = new SettingsPersistController(draft, bound);
    controllerRef.current = controller;
    const unregister = registerSettingsFlush(() => controller.flush());
    return () => {
      unregister();
      controller.dispose();
      if (controllerRef.current === controller) controllerRef.current = null;
    };
    // Recreate only when enabled / remounted — not on every draft tick.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    const controller = controllerRef.current;
    if (!controller) return;
    controller.setBlocked(turnBusy);
    controller.schedule(draft);
  }, [draft, enabled, turnBusy]);
}
