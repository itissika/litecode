import { WarningCircle } from "@phosphor-icons/react";
import type { ReactNode } from "react";

import type { AskAnswer, AskKind } from "../api/types";
import { actionButtonGlass, composerCardClass } from "./composerCard";
import type { Glyph } from "./ToolIcon";

/** Receipt of a human Ask. Only what the ask actually produced is set: a tool
 *  grant sends nothing, a plan approval may carry an opinion, an `ask_user`
 *  card carries the per-question answers. */
export interface PermissionGrantOpts {
  freeText?: string;
  /** Flat option ids — single-question compat only. */
  selected?: string[];
  /** Multi-question source of truth, keyed by question id. */
  answers?: Record<string, AskAnswer>;
}

export type Grant = (
  approved: boolean,
  always: boolean,
  opts?: PermissionGrantOpts,
) => void;

/** Single-line field inside an Ask card: no fill of its own (the card is the
 *  surface — only a hairline marks it), the action row's own height (30px — the
 *  row's buttons) and `btn-md`'s type size, so a field lines up with whatever row
 *  it shares. Callers add only their own layout (`flex-1` in the action row,
 *  `w-full` under a question). */
export const ASK_FIELD =
  "permission-card__input btn-md h-[30px] py-1 rounded-md border border-(--_dk-line) px-2.5 text-(--_dk-text-primary) placeholder:text-(--_dk-text-disabled)";

/** Geometry every Ask action button shares: the action row's own height (30px —
 *  what a field there must also match) and `btn-md`-ish type. */
const ASK_ACTION_SHAPE =
  "box-border flex h-[30px] shrink-0 items-center justify-center rounded-md border border-(--_dk-border-strong) px-2.5 text-[11px] leading-none text-(--_dk-text-primary)";

/** Decision button in the action row: the composer's own floating-button recipe
 *  (glass, hairline, 30px, same press feel) so the row of decisions and the row
 *  of controls under it read as one board — there is no second button language. */
export const ASK_BUTTON = `${actionButtonGlass} ${ASK_ACTION_SHAPE} transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90 disabled:cursor-not-allowed disabled:opacity-40 disabled:brightness-100`;

/** …and the same button with no fill of its own: the card's glass is the surface,
 *  as it is for the fields, so the hairline alone marks the button and hover only
 *  washes it. ask_user's Submit/Cancel wear this one — the glass recipe reads as a
 *  second surface inside a card that already is one. */
export const ASK_FLAT_BUTTON = `${ASK_ACTION_SHAPE} transition-[background-color,transform] duration-100 hover:bg-(--_dk-ix-bg-hover) active:scale-90 active:bg-(--_dk-ix-bg-pressed) disabled:cursor-not-allowed disabled:opacity-40`;

/** Step button parked in a card's title row (ask_user's question arrows): flat like
 *  ASK_FLAT_BUTTON, boxed to the title row's own height. */
export const ASK_STEP_BUTTON =
  "box-border flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[6px] border border-(--_dk-border-strong) text-(--_dk-text-primary) transition-[background-color,transform] duration-100 hover:bg-(--_dk-ix-bg-hover) active:scale-90 active:bg-(--_dk-ix-bg-pressed) disabled:cursor-not-allowed disabled:opacity-40";

/* ══ The board every Ask shares ═══════════════════════════════════════
   The composer dock's own frosted glass, a title row whose icon is the ask's own
   subject (it owns no column) and whose right end can carry the ask's own chrome,
   the ask's body, one bottom action row. The derived cards fill `children` +
   `actions` (and `trailing`, if they have a counter or a switch) and nothing else,
   so the surface, the title treatment and the row geometry cannot drift between
   asks — only what an ask has to say and offer changes. */
export function AskCard({
  kind,
  title,
  icon: Icon = WarningCircle,
  trailing,
  children,
  actions,
}: {
  /** Which ask this card carries — surfaced as `data-ask-kind` for CSS/tests. */
  kind: AskKind;
  /** What the ask is about, one line: a tool's own name, or the sentence naming
   *  the ask. */
  title: ReactNode;
  /** Glyph in the title row: the ask's own subject, so a tool ask shows that
   *  tool's icon. Sized and coloured here, so every title row matches. */
  icon?: Glyph;
  /** Right end of the title row — ask_user parks its question counter and step
   *  arrows there. Chrome only: the title keeps the row's own line. */
  trailing?: ReactNode;
  children?: ReactNode;
  /** Decision row: the buttons, plus any optional field riding with them. */
  actions: ReactNode;
}) {
  return (
    <div
      className={`${composerCardClass} permission-card flex shrink-0 flex-col gap-3 p-3`}
      role="group"
      aria-labelledby="perm-title"
      data-testid="permission-card"
      data-ask-kind={kind}
    >
      <div className="min-w-0">
        <div className="flex items-start justify-between gap-2">
          <h2
            id="perm-title"
            className="flex min-w-0 items-start gap-1.5 text-dk-lg font-semibold text-(--_dk-text-primary)"
          >
            <span
              aria-hidden
              className="mt-0.5 flex shrink-0 text-(--_dk-text-muted)"
            >
              <Icon size={14} weight="fill" />
            </span>
            <span className="min-w-0">{title}</span>
          </h2>
          {trailing}
        </div>
        {children}
      </div>

      <div className="flex flex-wrap items-center gap-2">{actions}</div>
    </div>
  );
}
