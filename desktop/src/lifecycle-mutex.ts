/**
 * Serialise async work so concurrent callers cannot interleave side effects
 * (e.g. stop+start of the same sidecar handle leaving an orphan).
 */
export class LifecycleMutex {
  private tail: Promise<void> = Promise.resolve();

  /** Run `fn` after all previously queued work settles (success or failure). */
  run<T>(fn: () => Promise<T>): Promise<T> {
    const next = this.tail.then(fn, fn);
    this.tail = next.then(
      () => undefined,
      () => undefined,
    );
    return next;
  }
}

/**
 * At most one in-flight job. Concurrent callers either reuse the same promise
 * or are rejected immediately.
 */
export class SingleFlight<T = unknown> {
  private inflight: Promise<T> | null = null;

  get busy(): boolean {
    return this.inflight !== null;
  }

  run(fn: () => Promise<T>, mode: "reuse" | "reject" = "reject"): Promise<T> {
    if (this.inflight) {
      if (mode === "reuse") return this.inflight;
      return Promise.reject(new Error("Operation already in progress."));
    }
    const job = fn().finally(() => {
      if (this.inflight === job) this.inflight = null;
    });
    this.inflight = job;
    return job;
  }
}
