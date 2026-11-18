import { CaretDownIcon } from "@phosphor-icons/react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { BashJob } from "../../api/types";
import { bashTail } from "../../lib/litecodeBash";

export function PinnedOutput({
  text,
  failed,
}: {
  text: string;
  failed: boolean;
}) {
  const preRef = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const el = preRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [text]);

  return (
    <div className="h-36 overflow-hidden">
      <pre
        ref={preRef}
        className={`h-full overflow-hidden whitespace-pre-wrap break-words px-2 py-1.5 font-mono text-dk-sm leading-relaxed ${
          failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-secondary)"
        }`}
      >
        {text}
      </pre>
    </div>
  );
}

export function CommandHeader({ command }: { command: string }) {
  const commandRef = useRef<HTMLSpanElement>(null);
  const [truncated, setTruncated] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const multiline = command.includes("\n");
  const expandable = multiline || truncated;

  useLayoutEffect(() => {
    if (expanded) return;
    const el = commandRef.current;
    if (!el) return;
    setTruncated(el.scrollWidth > el.clientWidth);
  }, [command, expanded]);

  return (
    <div className="shrink-0 bg-(--_dk-editor) px-2.5 py-2">
      <div className="flex items-start gap-1">
        <span
          ref={commandRef}
          className={`min-w-0 flex-1 font-mono text-dk-sm leading-relaxed text-(--_dk-text-body) ${
            expanded ? "whitespace-pre-wrap break-words" : "truncate"
          }`}
        >
          {command}
        </span>
        {expandable && (
          <button
            type="button"
            className="btn-ghost btn-icon btn-xs shrink-0"
            aria-label={expanded ? "Collapse command" : "Expand command"}
            aria-expanded={expanded}
            onClick={() => setExpanded((open) => !open)}
          >
            <CaretDownIcon
              size={12}
              className={`transition-transform duration-200 ${expanded ? "rotate-180" : ""}`}
              aria-hidden
            />
          </button>
        )}
      </div>
      <div
        className="mx-1 mt-2 border-b border-(--_dk-line-visible)"
        aria-hidden
      />
    </div>
  );
}

/**
 * Live background command tail monitor: polls `bash/tail` while alive or
 * draining and retains the final sampled tail after process exit.
 */
export function LiveTerminalView({
  job,
  command,
  failed = false,
  dataExecCallId,
}: {
  job: BashJob;
  command?: string;
  sessionId?: string;
  failed?: boolean;
  dataExecCallId?: string;
}) {
  const [tail, setTail] = useState<string>("");
  const [meta, setMeta] = useState<{
    alive: boolean;
    draining: boolean;
    truncatedOnDisk: boolean;
    exitCode: number | null;
  }>({
    alive: true,
    draining: false,
    truncatedOnDisk: false,
    exitCode: null,
  });

  useEffect(() => {
    let cancelled = false;
    let interval = 0;
    const poll = async (): Promise<boolean> => {
      try {
        const r = await bashTail(job.id);
        if (cancelled) return false;
        const draining = r.draining === true;
        setTail(r.text);
        setMeta({
          alive: r.alive,
          draining,
          truncatedOnDisk: r.truncated_on_disk,
          exitCode: r.exit_code,
        });
        return r.alive || draining;
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
  }, [job.id]);

  const running = meta.alive || meta.draining;
  const footerTone = running
    ? "text-(--_dk-text-muted)"
    : meta.exitCode === 0
      ? "text-(--_dk-text-muted)"
      : "text-(--_dk-amber-500)";

  return (
    <div
      className="overflow-hidden rounded-md border border-(--_dk-line-visible) bg-(--_dk-editor) font-mono text-dk-sm leading-relaxed"
      data-testid="live-terminal"
      data-exec-call-id={dataExecCallId}
    >
      {command !== undefined && <CommandHeader command={command} />}
      <PinnedOutput text={tail} failed={failed} />
      <span
        className={`block border-t border-(--_dk-line-visible) px-2 py-1 text-dk-xs ${footerTone}`}
        data-testid="live-terminal-footer"
      >
        {running
          ? "running"
          : meta.exitCode === null
            ? "exited"
            : `exit_code: ${meta.exitCode}`}
        {job.output_file ? ` · ${job.output_file}` : ""}
        {meta.truncatedOnDisk ? " · truncated on disk" : ""}
      </span>
    </div>
  );
}
