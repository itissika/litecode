/** Limited backoff for tunnel reconnect / sidecar restart (D1/D2). */

export type RecoveryPolicy = {
  /** Maximum automatic attempts after the first unexpected exit. */
  maxAttempts: number;
  /** Delay before attempt N (1-indexed). Falls back to last entry. */
  delaysMs: readonly number[];
};

export const DEFAULT_TUNNEL_RECOVERY: RecoveryPolicy = {
  maxAttempts: 3,
  delaysMs: [1_000, 2_000, 4_000],
};

export const DEFAULT_SIDECAR_RECOVERY: RecoveryPolicy = {
  maxAttempts: 2,
  delaysMs: [800, 1_600],
};

/** Whether an unexpected exit should trigger another auto-recovery attempt. */
export function shouldAutoRecover(
  intentional: boolean,
  attemptsSoFar: number,
  policy: RecoveryPolicy,
): boolean {
  if (intentional) return false;
  return attemptsSoFar < policy.maxAttempts;
}

/** Delay before the next recovery attempt (1-indexed attempt number). */
export function recoveryDelayMs(attempt: number, policy: RecoveryPolicy): number {
  if (attempt < 1) return policy.delaysMs[0] ?? 0;
  const idx = Math.min(attempt - 1, policy.delaysMs.length - 1);
  return policy.delaysMs[idx] ?? 0;
}

/**
 * Mutable counter used by main to track consecutive unexpected exits.
 * Reset on intentional stop or successful attach.
 */
export class RecoveryCounter {
  private attempts = 0;

  get count(): number {
    return this.attempts;
  }

  reset(): void {
    this.attempts = 0;
  }

  /**
   * Record one unexpected exit; returns whether an auto-recovery attempt should
   * be scheduled. Caps at `policy.maxAttempts` recoveries.
   */
  recordUnexpected(policy: RecoveryPolicy): boolean {
    if (this.attempts >= policy.maxAttempts) return false;
    this.attempts += 1;
    return true;
  }
}
