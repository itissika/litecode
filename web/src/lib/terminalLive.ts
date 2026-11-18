import {
  isFunctionCall,
  itemFromRow,
  parseFunctionArguments,
} from "../api/adapter";
import type { BashJob, HumanRow } from "../api/types";

export function formatElapsed(ms: number): string {
  const sec = Math.max(0, Math.floor(ms / 1000));
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}m${s}s`;
}

export function matchJobByCallId(
  jobs: BashJob[],
  callId: string | undefined,
): BashJob | undefined {
  if (!callId) return undefined;
  return jobs.find((j) => j.call_id === callId);
}

/**
 * Transcript-derived full command strings for `exec_command` calls keyed by `call_id`
 * (the live job snapshot only carries an 80-char `command_preview`).
 */
export function terminalCallCommandByCallId(
  rows: HumanRow[],
): Map<string, string> {
  const commands = new Map<string, string>();
  for (const row of rows) {
    const item = itemFromRow(row);
    if (!item || !isFunctionCall(item) || !item.call_id) continue;
    if (item.name !== "exec_command") continue;
    const args = parseFunctionArguments(item.arguments);
    if (args && typeof args === "object" && !Array.isArray(args)) {
      const cmd = (args as Record<string, unknown>).cmd;
      if (typeof cmd === "string") {
        commands.set(item.call_id, cmd);
      }
    }
  }
  return commands;
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
  names: readonly string[],
): boolean {
  for (const row of rows) {
    const item = itemFromRow(row);
    if (!item || !isFunctionCall(item) || item.call_id !== callId) continue;
    if (!names.includes(item.name)) continue;
    return true;
  }
  return false;
}

/**
 * Transcript target for the terminal panel's Reveal control.
 *
 * A nested `call_id` jumps to its parent `run_code` call; a top-level
 * `exec_command` / `write_stdin` call reveals that call card.
 */
export function terminalRevealCallId(
  callId: string,
  rows: HumanRow[],
): string | undefined {
  const parent = parentRunCodeCallId(callId);
  if (parent) {
    return hasFunctionCall(rows, parent, ["run_code"]) ? parent : undefined;
  }
  return hasFunctionCall(rows, callId, ["exec_command", "write_stdin"])
    ? callId
    : undefined;
}
