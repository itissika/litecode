import { useLayoutEffect, useRef, useState } from "react";

import type { AskKind, AskOption, AskQuestion } from "../api/types";
import { ASK_BUTTON, ASK_FIELD, AskCard, type Grant } from "./AskCard";
import { AskUserCard } from "./AskUserCard";
import { glyphFor } from "./ToolIcon";

/** The receipt of a human Ask lives with the board that shapes it; this module is
 *  where the ask cards are known from, so it stays reachable from here. */
export type { PermissionGrantOpts } from "./AskCard";

interface PermissionCardProps {
  tool: string;
  ruleId: string;
  summary: string;
  kind?: AskKind;
  freeText?: boolean;
  options?: AskOption[];
  multiSelect?: boolean;
  /** When set (ask_user), one card lists every question. */
  questions?: AskQuestion[];
  onGrant: Grant;
}

/* ── 1/3 · tool grant: Allow once / Always allow / Deny ───────────────
   The tool names itself — its own glyph and its id, like a tool row in the
   transcript — then the boundary that stopped the call, in plain words when it
   has a name and in none when it has not, then the call on one line. */

/** Command shapes worth reading twice: the destructive set the product itself
 *  knows (`src/tools/bash_safety.rs`) plus the usual foot-guns. A miss only
 *  costs the line — this is a hint for the human, never the rule. */
const RISKY_BASH: RegExp[] = [
  /^\s*sudo\b/,
  /\brm\s+-[a-z]*(r[a-z]*f|f[a-z]*r)/,
  /\b(mkfs\w*|dd)\b/,
  /\b(chown|chmod|pkill)\b/,
  /(curl|wget)[^|;&]*\|\s*\S*sh\b/,
];

/** The permission boundary the call ran into, in plain words — only the kinds
 *  worth saying out loud. Anything else (the catch-all rule, a rule written by
 *  hand) stays untranslated: no line beats a wrong one. */
function boundaryFor(tool: string, ruleId: string, summary: string): string | null {
  if (ruleId === "outside_workspace") return "Outside the workspace";
  if (tool === "bash" && RISKY_BASH.some((re) => re.test(summary)))
    return "Risky command";
  return null;
}

/** The call itself, on exactly one line whatever its length: cut off with a "!"
 *  tail and the whole text on hover. Never wraps, never grows the card. */
function AskCall({ text }: { text: string }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [cut, setCut] = useState(false);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setCut(el.scrollWidth > el.clientWidth);
    measure();
    if (typeof ResizeObserver === "undefined") return; // jsdom: measure once
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [text]);

  return (
    <p
      className="mt-0.5 flex items-baseline gap-1 font-mono text-dk-sm text-(--_dk-text-secondary)"
      title={cut ? text : undefined}
    >
      <span ref={ref} className="min-w-0 truncate">
        {text}
      </span>
      {cut && (
        <span aria-hidden className="shrink-0 text-(--_dk-text-muted)">
          !
        </span>
      )}
    </p>
  );
}

function PermissionAsk({
  tool,
  ruleId,
  summary,
  onGrant,
}: {
  tool: string;
  ruleId: string;
  summary: string;
  onGrant: Grant;
}) {
  const boundary = boundaryFor(tool, ruleId, summary);

  return (
    <AskCard
      kind="permission"
      icon={glyphFor(tool)}
      title={<span className="font-mono">{tool}</span>}
      actions={
        <>
          <button
            type="button"
            onClick={() => onGrant(true, false)}
            className={ASK_BUTTON}
          >
            Allow once
          </button>
          <button
            type="button"
            onClick={() => onGrant(true, true)}
            className={ASK_BUTTON}
          >
            Always allow
          </button>
          <button
            type="button"
            onClick={() => onGrant(false, false)}
            className={ASK_BUTTON}
          >
            Deny
          </button>
        </>
      }
    >
      {boundary && (
        <p className="mt-1.5 text-dk-base text-(--_dk-text-body)">{boundary}</p>
      )}
      <AskCall text={summary} />
    </AskCard>
  );
}

/* ── 2/3 · approval (plan create): Approve / Reject + opinion ─────────
   Headed by the tool's own glyph, like a tool ask. The ask's summary *is* the
   card, so it reads as prose; the optional opinion rides in the buttons' row
   instead of opening a text area of its own. */
function ApprovalAsk({
  tool,
  summary,
  freeText,
  onGrant,
}: {
  tool: string;
  summary: string;
  freeText: boolean;
  onGrant: Grant;
}) {
  const [opinion, setOpinion] = useState("");

  const submit = (approved: boolean) => {
    const text = freeText ? opinion.trim() || undefined : undefined;
    onGrant(approved, false, text ? { freeText: text } : undefined);
  };

  return (
    <AskCard
      kind="approval"
      icon={glyphFor(tool)}
      title={
        tool === "plan"
          ? "The agent wants to create a plan"
          : `The agent needs approval for ${tool}`
      }
      actions={
        <>
          <button
            type="button"
            onClick={() => submit(true)}
            className={ASK_BUTTON}
          >
            Approve
          </button>
          <button
            type="button"
            onClick={() => submit(false)}
            className={ASK_BUTTON}
          >
            Reject
          </button>
          {freeText && (
            <input
              type="text"
              data-testid="permission-free-text"
              aria-label="Opinion (optional)"
              placeholder="Opinion (optional)"
              className={`${ASK_FIELD} min-w-[8rem] flex-1`}
              value={opinion}
              onChange={(e) => setOpinion(e.target.value)}
            />
          )}
        </>
      }
    >
      <p className="mt-1.5 text-dk-base text-(--_dk-text-body)">{summary}</p>
    </AskCard>
  );
}

/** Entry point for every human Ask in the composer dock — the card that blocks
 *  the turn until the user answers. Routes by `kind` to the matching card: the
 *  two grants live here, `ask_user` in its own module; all three share the
 *  `AskCard` board. */
export function PermissionCard({
  tool,
  ruleId,
  summary,
  kind = "permission",
  freeText = false,
  options = [],
  multiSelect = false,
  questions,
  onGrant,
}: PermissionCardProps) {
  if (kind === "approval") {
    return (
      <ApprovalAsk
        tool={tool}
        summary={summary}
        freeText={freeText}
        onGrant={onGrant}
      />
    );
  }

  if (kind === "ask_user") {
    return (
      <AskUserCard
        tool={tool}
        summary={summary}
        options={options}
        multiSelect={multiSelect}
        freeText={freeText}
        questions={questions}
        onGrant={onGrant}
      />
    );
  }

  return (
    <PermissionAsk tool={tool} ruleId={ruleId} summary={summary} onGrant={onGrant} />
  );
}
