import { useState } from "react";
import { CaretDown, Robot } from "@phosphor-icons/react";

import type { MessageSource } from "../../api/types";
import i18n from "../../i18n";
import { botSourceName } from "../../lib/botMessage";

/**
 * Read-only sender marker for a bot-origin user message. It is a view control,
 * never an edit/revert/delete affordance.
 */
export function BotSourceMark({ sources }: { sources: MessageSource[] }) {
  const [open, setOpen] = useState(false);
  if (sources.length === 0) return null;
  const names = sources.map(botSourceName).join(", ");
  return (
    <div
      className="mb-1.5 flex flex-col gap-1"
      data-testid="bot-source-mark"
      onClick={(event) => event.stopPropagation()}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((value) => !value);
        }}
        className="flex max-w-full items-center gap-1 self-start rounded-md border border-(--_dk-line) bg-(--_dk-ix-bg-hover) px-1.5 py-0.5 text-[10px] text-(--_dk-text-secondary) transition-colors hover:text-(--_dk-text-primary)"
        title={i18n.t("bot.messageSource")}
      >
        <Robot size={11} aria-hidden />
        <span className="min-w-0 truncate">
          {i18n.t("bot.messageFrom", { name: names })}
        </span>
        <CaretDown
          size={10}
          aria-hidden
          className={open ? "rotate-180 transition-transform" : "transition-transform"}
        />
      </button>
      {open ? (
        <div
          role="note"
          className="rounded-md border border-(--_dk-line) bg-(--_dk-ix-bg-hover) px-2 py-1 text-[10px] text-(--_dk-text-muted)"
        >
          {sources.map((source) => (
            <div key={source.id} className="flex min-w-0 gap-1">
              <span className="shrink-0 text-(--_dk-text-secondary)">
                {botSourceName(source)}
              </span>
              <span className="min-w-0 truncate font-mono">{source.id}</span>
            </div>
          ))}
          <p className="mt-1">{i18n.t("bot.messageReadOnly")}</p>
        </div>
      ) : null}
    </div>
  );
}
