import type { BashJob } from "../api/types";

export function formatElapsed(ms: number): string {
  const sec = Math.max(0, Math.floor(ms / 1000));
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}m${s}s`;
}

export function parseBashId(text: string): string | undefined {
  const m = /^bash_id:\s*(\S+)/m.exec(text);
  return m?.[1];
}

export function isRunningStatusText(text: string): boolean {
  return /^status:\s*running\b/m.test(text);
}

/**
 * Leading `exit_code: N` line of a completed bash view. The backend puts it on
 * the FIRST line of the document, so only the document start counts.
 */
export function headExitCode(text: string): number | null {
  const m = /^exit_code:\s*(-?\d+)\s*(?:\n|$)/.exec(text);
  return m ? Number(m[1]) : null;
}

/**
 * A bash tool result that came back as a BACKGROUND job: the backend seals it as
 * `status: running` + `bash_id:` (+ the running list) and never rewrites it when
 * the process later exits. Pure text check — usable where no store is in scope
 * (e.g. transcript routing).
 */
export function isBackgroundBashResult(text: string): boolean {
  return isRunningStatusText(text) || parseBashId(text) !== undefined;
}

/**
 * Live verdict for a sealed background-bash result.
 *
 * Termination condition: the text alone can never settle — `status: running` is a
 * one-way seal. The `/bash/jobs` snapshot is the tiebreaker, so a job that has
 * left the snapshot ends the claim: the process exited (or is no longer tracked)
 * and the card/row must settle instead of showing a frozen tail forever.
 */
export function isBashJobLive(text: string, job: BashJob | undefined): boolean {
  return job !== undefined && isRunningStatusText(text);
}

export function matchJob(
  jobs: BashJob[],
  callId: string | undefined,
  outputText: string,
): BashJob | undefined {
  if (callId) {
    const byCall = jobs.find((j) => j.call_id === callId);
    if (byCall) return byCall;
  }
  const bashId = parseBashId(outputText);
  if (bashId) return jobs.find((j) => j.id === bashId);
  return undefined;
}
