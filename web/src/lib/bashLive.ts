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
