import { CaretDownIcon } from "@phosphor-icons/react";
import { useLayoutEffect, useRef, useState } from "react";

import { functionCallOutputText } from "../../api/adapter";
import { ToolInfoIcon } from "./InfoIcon";
import { collectMetaFields, TOOL_PARAM_META } from "./paramMeta";
import type { ToolViewProps } from "./registry";
import {
  parseSessionSearch,
  type SessionSearchFooter,
  type SessionSearchGroup,
  type SessionSearchHit,
} from "./sessionSearchParse";

const RAW_PRE =
  "whitespace-pre-wrap break-words font-mono text-dk-sm leading-relaxed";

function collapsePreview(lines: string[]): string {
  return lines.join(" ").replace(/\s+/g, " ").trim();
}

function lineLabel(hit: SessionSearchHit): string {
  return hit.to === undefined ? `L${hit.from}` : `L${hit.from}-${hit.to}`;
}

function matchLabel(count: number): string {
  return count === 1 ? "1 match" : `${count} matches`;
}

function inputObject(input: unknown): Record<string, unknown> {
  if (!input || typeof input !== "object" || Array.isArray(input)) return {};
  return input as Record<string, unknown>;
}

function ScopeRow({
  sessionId,
  meta,
}: {
  sessionId: string | undefined;
  meta: ReturnType<typeof collectMetaFields>;
}) {
  if (!sessionId && meta.length === 0) return null;
  return (
    <div
      className="flex items-center gap-1.5"
      data-testid="session-search-scope"
    >
      {sessionId && (
        <span className="min-w-0 truncate font-mono text-dk-xs text-(--_dk-text-muted)">
          session {sessionId}
        </span>
      )}
      <ToolInfoIcon fields={meta} />
    </div>
  );
}

function RawBlock({ text, failed }: { text: string; failed: boolean }) {
  return (
    <pre
      className={`${RAW_PRE} ${
        failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-secondary)"
      }`}
      data-testid="session-search-raw"
    >
      {text}
    </pre>
  );
}

function HitRow({ hit }: { hit: SessionSearchHit }) {
  const previewRef = useRef<HTMLSpanElement>(null);
  const [truncated, setTruncated] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const preview = collapsePreview(hit.lines);
  const multiline = hit.lines.length > 1;
  const expandable = multiline || truncated;

  useLayoutEffect(() => {
    const el = previewRef.current;
    if (!el) return;
    setTruncated(el.scrollWidth > el.clientWidth);
  }, [preview]);

  const rowClass = "flex w-full min-w-0 items-baseline gap-2 px-1.5 py-1 text-left";
  const body = (
    <>
      <span className="shrink-0 select-none font-mono text-dk-xs tabular-nums text-(--_dk-text-disabled)">
        {lineLabel(hit)}
      </span>
      <span className="max-w-[40%] shrink-0 truncate font-mono text-dk-xs text-(--_dk-text-muted)">
        {hit.label}
      </span>
      <span
        ref={previewRef}
        className="min-w-0 flex-1 truncate font-mono text-dk-xs text-(--_dk-text-secondary)"
        data-testid="session-search-preview"
      >
        {preview}
      </span>
      {expandable && (
        <CaretDownIcon
          size={12}
          className={`shrink-0 text-(--_dk-text-disabled) transition-transform duration-200 ${
            expanded ? "rotate-180" : ""
          }`}
          aria-hidden
        />
      )}
    </>
  );

  return (
    <li data-testid="session-search-hit">
      {expandable ? (
        <button
          type="button"
          className={`${rowClass} hover:bg-(--_dk-ix-bg-hover)`}
          aria-expanded={expanded}
          onClick={() => setExpanded((open) => !open)}
        >
          {body}
        </button>
      ) : (
        <div className={rowClass}>{body}</div>
      )}
      {expanded && (
        <pre
          className="whitespace-pre-wrap break-words px-1.5 pb-1 font-mono text-dk-xs leading-relaxed text-(--_dk-text-secondary)"
          data-testid="session-search-hit-body"
        >
          {hit.lines.join("\n")}
        </pre>
      )}
    </li>
  );
}

function GroupBlock({ group }: { group: SessionSearchGroup }) {
  return (
    <li data-testid="session-search-group">
      <div className="flex items-baseline gap-2 px-1.5 pt-1">
        <span className="min-w-0 truncate font-mono text-dk-xs text-(--_dk-text-muted)">
          {group.handle} · {group.age}
        </span>
        <span
          className="ml-auto shrink-0 pl-1 text-dk-2xs text-(--_dk-text-disabled)"
          data-testid="session-search-count"
        >
          {matchLabel(group.count)}
        </span>
      </div>
      <ul>
        {group.hits.map((hit, index) => (
          <HitRow key={`${hit.from}-${hit.to ?? ""}-${index}`} hit={hit} />
        ))}
      </ul>
    </li>
  );
}

function FooterNote({ footer }: { footer: SessionSearchFooter }) {
  return (
    <div
      className="px-1.5 pt-1 text-dk-2xs text-(--_dk-text-muted)"
      data-testid="session-search-footer"
    >
      <p>
        Showing {footer.shown} of {footer.total}
        {footer.location ? (
          <>
            {" "}
            · rest in <span className="font-mono break-all">{footer.location}</span>
          </>
        ) : null}
      </p>
      {footer.more ? <p className="break-words">More in: {footer.more}</p> : null}
    </div>
  );
}

/**
 * Expanded body of `session_search`. The fold-card title already shows the
 * query; this view is the result list. Output that is not the agent grammar
 * stays a raw pre, same as the default renderer.
 */
export function SessionSearchToolView({
  name,
  status,
  input,
  output,
}: ToolViewProps) {
  const raw = output ? functionCallOutputText(output) : "";
  const failed = status === "failed";
  const sessionIdRaw = inputObject(input).session_id;
  const sessionId =
    typeof sessionIdRaw === "string" && sessionIdRaw.trim()
      ? sessionIdRaw.trim()
      : undefined;
  const meta = collectMetaFields(
    input,
    TOOL_PARAM_META[name]?.primary ?? ["query", "session_id"],
  );
  const scope =
    sessionId || meta.length > 0 ? (
      <ScopeRow sessionId={sessionId} meta={meta} />
    ) : null;

  if (failed) {
    return (
      <div className="flex flex-col gap-1" data-testid="session-search-view">
        {scope}
        {raw ? <RawBlock text={raw} failed /> : null}
      </div>
    );
  }

  const view = raw ? parseSessionSearch(raw) : null;
  if (!view) {
    if (!raw && !scope) return null;
    return (
      <div className="flex flex-col gap-1" data-testid="session-search-view">
        {scope}
        {raw ? <RawBlock text={raw} failed={false} /> : null}
      </div>
    );
  }

  if (view.kind === "empty") {
    return (
      <div className="flex flex-col gap-1" data-testid="session-search-view">
        {scope}
        <p
          className={`${RAW_PRE} text-(--_dk-text-muted)`}
          data-testid="session-search-empty"
        >
          {raw}
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1" data-testid="session-search-view">
      {scope}
      <ul>
        {view.groups.map((group, index) => (
          <GroupBlock key={`${group.handle}-${index}`} group={group} />
        ))}
      </ul>
      {view.footer ? <FooterNote footer={view.footer} /> : null}
    </div>
  );
}
