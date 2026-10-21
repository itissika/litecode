import {
  functionCallOutputText,
  isFunctionCall,
  isFunctionCallOutput,
  itemFromRow,
  parseFunctionArguments,
} from "../api/adapter";
import type { BashJob, FunctionCallOutputItem, HumanRow } from "../api/types";

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

/**
 * Transcript-derived metadata for one bash tool call, keyed by `call_id`.
 *
 *  - `command` — the full command from the call arguments (the job wire only
 *    carries a collapsed, 80-char `command_preview`).
 *  - `output` — the sealed tool result, when the result row is loaded.
 *  - `background` — the capsule's ownership verdict: `run_in_background: true`
 *    on the call, or a result that sealed as a running document (a foreground
 *    call the backend converted to a job when it outlived its wait — the
 *    transcript routes it to the single-line row on the same text).
 *
 * A call outside the loaded window has no entry; callers treat that as
 * background (unknown), which is the safe direction — a live job's call row is
 * in the window in practice, and hiding a real terminal is worse than showing
 * a conservative one.
 */
export interface BashCallMeta {
  command?: string;
  output?: FunctionCallOutputItem;
  background: boolean;
}

export function bashCallMetaByCallId(
  rows: HumanRow[],
): Map<string, BashCallMeta> {
  const meta = new Map<string, BashCallMeta>();
  for (const row of rows) {
    const item = itemFromRow(row);
    if (!item || !isFunctionCall(item)) continue;
    if (item.name !== "bash" || !item.call_id) continue;
    const args = parseFunctionArguments(item.arguments);
    const obj =
      args && typeof args === "object" && !Array.isArray(args)
        ? (args as Record<string, unknown>)
        : {};
    const entry: BashCallMeta = { background: obj.run_in_background === true };
    if (typeof obj.command === "string") entry.command = obj.command;
    meta.set(item.call_id, entry);
  }
  for (const row of rows) {
    const item = itemFromRow(row);
    if (!item || !isFunctionCallOutput(item) || !item.call_id) continue;
    const entry = meta.get(item.call_id);
    if (!entry) continue;
    entry.output = item;
    if (isBackgroundBashResult(functionCallOutputText(item))) {
      entry.background = true;
    }
  }
  return meta;
}

const NESTED_CODE_SUFFIX = /:code:\d+$/;

/** Parent `run_code` call id for a nested code call (`parent:code:0`). */
export function parentRunCodeCallId(callId: string): string | undefined {
  const match = NESTED_CODE_SUFFIX.exec(callId);
  if (!match || match.index === 0) return undefined;
  return callId.slice(0, match.index);
}

function hasFunctionCall(
  rows: HumanRow[],
  callId: string,
  name?: string,
): boolean {
  for (const row of rows) {
    const item = itemFromRow(row);
    if (!item || !isFunctionCall(item) || item.call_id !== callId) continue;
    if (name !== undefined && item.name !== name) continue;
    return true;
  }
  return false;
}

/**
 * Transcript target for the terminal panel's Reveal control.
 *
 * Jobs are listed from the snapshot, not from this lookup. Reveal is hidden
 * when the transcript has no row that can be scrolled to. A nested `call_id`
 * jumps to the parent `run_code` call and is never itself the DOM id.
 * A historical `bash` call reveals that call.
 */
export function terminalRevealCallId(
  callId: string,
  rows: HumanRow[],
): string | undefined {
  const parent = parentRunCodeCallId(callId);
  if (parent) {
    return hasFunctionCall(rows, parent, "run_code") ? parent : undefined;
  }
  return hasFunctionCall(rows, callId, "bash") ? callId : undefined;
}
