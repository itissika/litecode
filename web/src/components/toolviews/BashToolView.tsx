import { useEffect, useRef, useState } from "react";

import { functionCallOutputText } from "../../api/adapter";
import { bashTail } from "../../lib/litecodeBash";
import { isRunningStatusText, matchJob } from "../../lib/bashLive";
import { useBashStore } from "../../stores/bashStore";
import type { ToolViewProps } from "./registry";

interface ParsedBashOutput {
  stdout: string;
  stderr: string;
  exitCode: string | null;
}

/**
 * Backend serializes bash output as a single string:
 *   <stdout>\nstderr:\n<stderr>\nexit_code: N
 * Split it back apart for per-stream coloring. The stderr block and exit_code
 * line are optional.
 */
function parseBashOutput(raw: string): ParsedBashOutput {
  let body = raw;
  let exitCode: string | null = null;

  const exitMatch = /\nexit_code:\s*(-?\d+)\s*$/.exec(body);
  if (exitMatch) {
    exitCode = exitMatch[1];
    body = body.slice(0, exitMatch.index);
  }

  const marker = "\nstderr:\n";
  const idx = body.indexOf(marker);
  if (idx >= 0) {
    return {
      stdout: body.slice(0, idx),
      stderr: body.slice(idx + marker.length),
      exitCode,
    };
  }
  return { stdout: body, stderr: "", exitCode };
}

function ConsoleBody({
  text,
  failed,
  footer,
  stick,
}: {
  text: string;
  failed: boolean;
  footer?: string;
  stick?: boolean;
}) {
  const preRef = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (!stick) return;
    const el = preRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [stick, text]);

  return (
    <div className="max-h-60 overflow-auto rounded-md border border-(--_dk-line-visible) font-mono text-dk-sm leading-relaxed">
      <pre
        ref={preRef}
        className={`max-h-60 overflow-auto whitespace-pre-wrap break-words px-2 py-1.5 ${
          failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-secondary)"
        }`}
      >
        {text}
      </pre>
      {footer !== undefined && (
        <span className="block border-t border-(--_dk-line-visible) px-2 py-1 text-dk-xs text-(--_dk-text-muted)">
          {footer}
        </span>
      )}
    </div>
  );
}

/**
 * Bash tool body: command + stdout/stderr, or a live tee tail overlay while
 * the process is still running after the tool result sealed.
 */
export function BashToolView({ status, input, output, call_id, sessionId }: ToolViewProps) {
  const obj =
    input && typeof input === "object" && !Array.isArray(input)
      ? (input as Record<string, unknown>)
      : {};
  const command = typeof obj.command === "string" ? obj.command : undefined;

  const rawOutput = output ? functionCallOutputText(output) : "";
  const parsed = rawOutput ? parseBashOutput(rawOutput) : null;
  const failed = status === "failed";

  const job = useBashStore((s) => {
    if (!sessionId) return undefined;
    const jobs = s.bySession.get(sessionId)?.jobs ?? [];
    return matchJob(jobs, call_id, rawOutput);
  });

  const [tail, setTail] = useState<string | null>(null);
  const [tailMeta, setTailMeta] = useState<{
    alive: boolean;
    exitCode: number | null;
  }>({ alive: true, exitCode: null });

  useEffect(() => {
    if (!job) return;
    let cancelled = false;
    let interval = 0;
    const poll = async (): Promise<boolean> => {
      try {
        const r = await bashTail(job.id);
        if (cancelled) return false;
        setTail((prev) => (prev === r.text ? prev : r.text));
        setTailMeta({ alive: r.alive, exitCode: r.exit_code });
        return r.alive;
      } catch {
        return true;
      }
    };
    void poll().then((keep) => {
      if (cancelled || !keep) return;
      interval = window.setInterval(() => {
        void poll().then((still) => {
          if (!still) window.clearInterval(interval);
        });
      }, 250);
    });
    return () => {
      cancelled = true;
      if (interval) window.clearInterval(interval);
    };
  }, [job?.id]);

  const runningSealed = isRunningStatusText(rawOutput);
  const showLive = tail !== null && (Boolean(job) || runningSealed);
  const liveFooter =
    job || tailMeta.alive
      ? undefined
      : tailMeta.exitCode !== null
        ? `exited  exit_code: ${tailMeta.exitCode}`
        : "exited";

  return (
    <div className="flex flex-col gap-2">
      {command !== undefined && (
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-md border border-(--_dk-line-visible) bg-(--_dk-surface-header) px-2 py-1.5 font-mono text-dk-sm leading-relaxed text-(--_dk-text-body)">
          {command}
        </pre>
      )}

      {showLive ? (
        <ConsoleBody text={tail ?? ""} failed={failed} footer={liveFooter} stick />
      ) : (
        parsed &&
        (parsed.stdout || parsed.stderr || parsed.exitCode !== null) && (
          <div className="max-h-60 overflow-auto rounded-md border border-(--_dk-line-visible) font-mono text-dk-sm leading-relaxed">
            {parsed.stdout && (
              <pre
                className={`whitespace-pre-wrap break-words px-2 py-1.5 ${
                  failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-secondary)"
                }`}
              >
                {parsed.stdout}
              </pre>
            )}
            {parsed.stderr && (
              <pre className="whitespace-pre-wrap break-words border-t border-(--_dk-line-visible) bg-(--_dk-amber-500)/5 px-2 py-1.5 text-(--_dk-amber-500)">
                {parsed.stderr}
              </pre>
            )}
            {parsed.exitCode !== null && (
              <span
                className={`block border-t border-(--_dk-line-visible) px-2 py-1 text-dk-xs ${
                  parsed.exitCode === "0"
                    ? "text-(--_dk-text-muted)"
                    : "text-(--_dk-amber-500)"
                }`}
              >
                exit_code: {parsed.exitCode}
              </span>
            )}
          </div>
        )
      )}
    </div>
  );
}
