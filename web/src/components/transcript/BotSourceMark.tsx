import type { ReactNode } from "react";
import { CaretDown, Robot, X } from "@phosphor-icons/react";

import type { MessageSource } from "../../api/types";
import i18n from "../../i18n";
import { botSourceName } from "../../lib/botMessage";
import { Popover } from "../ui/Popover";

/**
 * Collapsed bot-origin user message. The transcript shows only the sender line;
 * the message body and sender details live in the shared popover. It is a view
 * control, never an edit/revert/delete affordance.
 */
export function BotSourceMark({
  sources,
  children,
}: {
  sources: MessageSource[];
  children?: ReactNode;
}) {
  if (sources.length === 0) return null;
  const names = sources.map(botSourceName).join(", ");
  return (
    <Popover
      className="block w-full"
      placement="up-right"
      width={440}
      gap={6}
      panelClassName="max-h-96 overflow-y-auto"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          data-testid="bot-message-mark"
          aria-expanded={open}
          title={i18n.t("bot.messageShow")}
          onClick={(event) => {
            event.stopPropagation();
            toggle();
          }}
          className="group flex w-full cursor-pointer select-none items-center gap-1.5 rounded py-1 text-left transition-colors hover:bg-(--_dk-ix-bg-hover)"
        >
          <Robot
            size={11}
            aria-hidden
            className="shrink-0 text-(--_dk-text-secondary)"
          />
          <span className="min-w-0 truncate text-dk-2xs text-(--_dk-text-disabled) group-hover:text-(--_dk-text-secondary)">
            {i18n.t("bot.messageFrom", { name: names })}
          </span>
          <CaretDown
            size={10}
            aria-hidden
            className={`shrink-0 text-(--_dk-text-disabled) transition-transform ${open ? "rotate-180" : ""}`}
          />
        </button>
      )}
    >
      {({ close }) => (
        <div className="p-3">
          <div className="mb-2 flex items-center justify-between gap-2">
            <span className="text-dk-2xs font-medium text-(--_dk-text-secondary)">
              {i18n.t("bot.messageTitle")}
            </span>
            <button
              type="button"
              onClick={close}
              aria-label={i18n.t("bot.messageClose")}
              className="flex h-4 w-4 shrink-0 items-center justify-center rounded text-(--_dk-text-muted) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary)"
            >
              <X size={10} weight="bold" />
            </button>
          </div>
          <div className="mb-2 flex flex-col gap-1 text-[10px] text-(--_dk-text-muted)">
            {sources.map((source) => (
              <div key={source.id} className="flex min-w-0 gap-1">
                <span className="shrink-0 text-(--_dk-text-secondary)">
                  {botSourceName(source)}
                </span>
                <span className="min-w-0 truncate font-mono">{source.id}</span>
              </div>
            ))}
          </div>
          {children ? (
            <div className="border-t border-(--_dk-line) pt-2">
              {children}
            </div>
          ) : null}
          <p className="mt-2 text-[10px] text-(--_dk-text-muted)">
            {i18n.t("bot.messageReadOnly")}
          </p>
        </div>
      )}
    </Popover>
  );
}
