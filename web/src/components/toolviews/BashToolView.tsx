import { CaretDownIcon } from "@phosphor-icons/react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { functionCallOutputText } from "../../api/adapter";
import { bashTail } from "../../lib/litecodeBash";
import { matchJob, parseBashId } from "../../lib/bashLive";
import { useBashStore } from "../../stores/bashStore";
import type { ToolViewProps } from "./registry";

/**

/**
 * The REAL backend shapes (see `src/tools/bash_status.rs`):
 *
 *  - running view   — `status: running` / `bash_id: …` / `output_file: …` /
 *                     the running list / guidance. NO captured output.
 *  - completed view — `exit_code: N` (or `status: cancelled`) FIRST, then either
 *                     the output itself (small capture) or a frozen window:
 *                     `bytes: N`, `output_file: …`, `truncated_on_disk: true`,
 *                     a `[head … + tail … of N bytes]` note, then
 *                     `--- head ---` <head> `--- tail ---` <tail>.
 *
 * There is no `stderr:` section anywhere in the format: the backend tees the
 * merged stdout+stderr stream, so per-stream colouring cannot be derived from
 * the text. The two-tone design is kept for the real sections instead — the
 * pinned head window, an amber trailing window, and the exit-code footer.
 */
interface ParsedBashOutput {
  /** `status: running` — a live background job; the text carries no output. */
  running: boolean;
  /** `status: cancelled`. */
  cancelled: boolean;
  /** Leading `exit_code: N` of a completed view. */
  exitCode: string | null;
  /** Head window — the whole captured output when it was never frozen. */
  head: string;
  /** Trailing window; only present when the capture froze into head+tail. */
  tail: string | null;
  /** `bytes: N` of a frozen capture. */
  bytes: number | null;
  /** `output_file:` pointer into the workspace. */
  outputFile: string | null;
  truncatedOnDisk: boolean;
}

const HEAD_MARK = "--- head ---";
const TAIL_MARK = "--- tail ---";

function emptyParsed(): ParsedBashOutput {
  return {
    running: false,
    cancelled: false,
    exitCode: null,
    head: "",
    tail: null,
    bytes: null,
    outputFile: null,
    truncatedOnDisk: false,
  };
}

/** Line-based parse of the backend's bash result document. */
function parseBashOutput(raw: string): ParsedBashOutput {
  const out = emptyParsed();
  const lines = raw.split("\n");
  let i = 0;

  const status = /^status:\s*(\S+)/.exec(lines[i] ?? "");
  if (status) {
    out.running = status[1] === "running";
    out.cancelled = status[1] === "cancelled";
    i += 1;
  }
  const code = /^exit_code:\s*(-?\d+)/.exec(lines[i] ?? "");
  if (code) {
    out.exitCode = code[1]!;
    i += 1;
  }

  if (out.running) {
    // Status document only: keep the durable pointer to the full log.
    for (; i < lines.length; i += 1) {
      const file = /^output_file:\s*(\S+)/.exec(lines[i]!);
      if (file) out.outputFile = file[1]!;
    }
    return out;
  }

  const headAt = lines.indexOf(HEAD_MARK, i);
  if (headAt >= 0) {
    for (; i < headAt; i += 1) {
      const line = lines[i]!;
      const bytes = /^bytes:\s*(\d+)/.exec(line);
      if (bytes) out.bytes = Number(bytes[1]);
      const file = /^output_file:\s*(\S+)/.exec(line);
      if (file) out.outputFile = file[1]!;
      if (line.startsWith("truncated_on_disk:")) out.truncatedOnDisk = true;
    }
    const tailAt = lines.indexOf(TAIL_MARK, headAt + 1);
    out.head = stripTrailingNewline(
      lines.slice(headAt + 1, tailAt < 0 ? undefined : tailAt).join("\n"),
    );
    out.tail = stripTrailingNewline(
      tailAt < 0 ? "" : lines.slice(tailAt + 1).join("\n"),
    );
    return out;
  }

  // Small capture: everything after the leading line(s) is the output.
  out.head = stripTrailingNewline(lines.slice(i).join("\n"));
  return out;
}

function stripTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text;
}

function PinnedOutput({ text, failed }: { text: string; failed: boolean }) {
  const preRef = useRef<HTMLPreElement>(null);
  // Bash output is a fixed tail window by design (no scroll-up): always pin to
  // the newest line. Earlier output is read in the enclosing FoldCard scroller.
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

function CommandHeader({ command }: { command: string }) {
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
 * Bash tool body: command + the captured output, or a live tee tail overlay
 * while the process is still running after the tool result sealed.
 */
export function BashToolView({
  status,
  input,
  output,
  call_id,
  sessionId,
}: ToolViewProps) {
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

  // The tee window is polled per BASH ID, not per job object: the id survives the
  // job leaving the `/bash/jobs` snapshot, so the poll keeps running to the real
  // process exit instead of freezing on the last sample taken before it vanished.
  const bashId = job?.id ?? parseBashId(rawOutput);

  const [tail, setTail] = useState<string | null>(null);
  const [tailMeta, setTailMeta] = useState<{
    alive: boolean;
    exitCode: number | null;
  }>({ alive: true, exitCode: null });

  useEffect(() => {
    if (!bashId) return;
    let cancelled = false;
    let interval = 0;
    const poll = async (): Promise<boolean> => {
      try {
        const r = await bashTail(bashId);
        if (cancelled) return false;
        setTailMeta({ alive: r.alive, exitCode: r.exit_code });
        // Exit: drop the overlay so the card settles on the sealed document
        // (which points at the full log) plus the real exit code below.
        setTail(r.alive ? r.text : null);
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
  }, [bashId]);

  // Live overlay is bounded by the PROCESS, not by the job snapshot: `tail` is
  // null until the first sample and is cleared the moment the poll reports exit.
  const showLive = tail !== null && tailMeta.alive;

  const hasSealedOutput =
    parsed !== null &&
    (parsed.head !== "" ||
      parsed.tail !== null ||
      parsed.exitCode !== null ||
      parsed.cancelled ||
      parsed.outputFile !== null);

  // Settled footer: the sealed document's own verdict first, then whatever the
  // poll learned before the overlay ended (a background job's sealed text has
  // no exit code — the poll is the only source of the real outcome).
  let footer: string | undefined;
  let footerTone = "text-(--_dk-text-muted)";
  if (parsed?.cancelled) {
    footer = "status: cancelled";
    footerTone = "text-(--_dk-amber-500)";
  } else if (parsed && parsed.exitCode !== null) {
    footer = `exit_code: ${parsed.exitCode}`;
    footerTone =
      parsed.exitCode === "0"
        ? "text-(--_dk-text-muted)"
        : "text-(--_dk-amber-500)";
  } else if (tailMeta.alive === false) {
    footer =
      tailMeta.exitCode === null
        ? "exited"
        : `exited  exit_code: ${tailMeta.exitCode}`;
    footerTone =
      tailMeta.exitCode === 0
        ? "text-(--_dk-text-muted)"
        : "text-(--_dk-amber-500)";
  }

  if (command === undefined && !showLive && !hasSealedOutput) {
    return null;
  }

  return (
    <div
      className="overflow-hidden rounded-md border border-(--_dk-line-visible) bg-(--_dk-editor) font-mono text-dk-sm leading-relaxed"
      data-testid="bash-console"
      data-bash-call-id={call_id}
    >
      {command !== undefined && <CommandHeader command={command} />}

      {showLive ? (
        <PinnedOutput text={tail ?? ""} failed={failed} />
      ) : (
        hasSealedOutput && (
          <>
            {parsed!.head && (
              <PinnedOutput text={parsed!.head} failed={failed} />
            )}
            {parsed!.tail !== null && (
              <>
                <span
                  className="block border-t border-(--_dk-line-visible) px-2 py-1 text-dk-xs text-(--_dk-text-muted)"
                  data-testid="bash-window-note"
                >
                  windowed head + tail
                  {parsed!.bytes !== null ? ` · ${parsed!.bytes} bytes` : ""}
                  {parsed!.outputFile ? ` · ${parsed!.outputFile}` : ""}
                  {parsed!.truncatedOnDisk ? " · truncated on disk" : ""}
                </span>
                {parsed!.tail && (
                  <pre
                    className="max-h-36 overflow-hidden whitespace-pre-wrap break-words border-t border-(--_dk-line-visible) bg-(--_dk-amber-500)/5 px-2 py-1.5 text-(--_dk-amber-500)"
                    data-testid="bash-tail"
                  >
                    {parsed!.tail}
                  </pre>
                )}
              </>
            )}
            {!parsed!.head && parsed!.outputFile && (
              <span
                className="block px-2 py-1.5 text-dk-xs text-(--_dk-text-muted)"
                data-testid="bash-output-file"
              >
                {parsed!.outputFile}
              </span>
            )}
            {footer !== undefined && (
              <span
                className={`block border-t border-(--_dk-line-visible) px-2 py-1 text-dk-xs ${footerTone}`}
                data-testid="bash-footer"
              >
                {footer}
              </span>
            )}
          </>
        )
      )}
    </div>
  );
}
