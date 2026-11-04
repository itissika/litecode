import type { HTMLAttributes, ReactNode } from "react";
import { Trash } from "@phosphor-icons/react";

import type { TurnStepKind } from "../api/types";
import { LivePreview } from "./LivePreview";
import { InlineRename } from "./ui/InlineRename";
import { ListRow } from "./ui/ListRow";
import { SessionStatusDot, type SessionStatus } from "./SessionStatusDot";
import i18n from "../i18n";

export interface SidebarRowProps {
  /** Visible label; the caller resolves title override / bot name. */
  label: string;
  /** Full tooltip; defaults to the label. */
  title?: string;
  status: SessionStatus;
  unread: boolean;
  focused: boolean;
  relative: string;
  stepKinds?: TurnStepKind[];
  running: boolean;
  updatedAt: number;
  now: number;
  indentPx?: number;
  /** Optional glyph between the status bar and the label (bot robot). */
  icon?: ReactNode;
  /** Drag handlers from useDragSource; spread onto the clickable row. */
  drag?: HTMLAttributes<HTMLElement>;
  /** Drop handlers from useDropTarget; row-to-row grouping. */
  dropHandlers?: HTMLAttributes<HTMLElement>;
  dropPosition?: "before" | "inside" | "after" | null;
  /** Extra data-* attributes for tests / selection CSS. */
  dataAttrs?: Record<string, string | undefined>;
  onOpen: () => void;
  onDelete: () => void;
  deleteBlocked: boolean;
  deleteLabel: string;
  onContextMenu?: (event: React.MouseEvent) => void;
  renaming: boolean;
  renameInitialValue: string;
  renameAriaLabel: string;
  renameMaxLength: number;
  renameAllowEmpty?: boolean;
  onStartRename?: () => void;
  onRename: (value: string) => void;
  onRenameCancel: () => void;
}

/**
 * One sidebar row shared by sessions and bots. Selection, unread/running
 * presentation, hover time, delete slot, context menu, drag, and inline rename
 * are intentionally identical for both product identities.
 */
export function SidebarRow({
  label,
  title,
  status,
  unread,
  focused,
  relative,
  stepKinds,
  running,
  updatedAt,
  now,
  indentPx = 12,
  icon,
  drag,
  dropHandlers,
  dropPosition = null,
  dataAttrs,
  onOpen,
  onDelete,
  deleteBlocked,
  deleteLabel,
  onContextMenu,
  renaming,
  renameInitialValue,
  renameAriaLabel,
  renameMaxLength,
  renameAllowEmpty = false,
  onStartRename,
  onRename,
  onRenameCancel,
}: SidebarRowProps) {
  return (
    <div className="group relative h-9" {...dataAttrs}>
      <ListRow
        size="session"
        inset={indentPx}
        className={`cursor-pointer text-(--_dk-ix-fg) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary) active:bg-(--_dk-ix-bg-pressed) focus-visible:shadow-[0_0_0_2px_var(--_dk-ix-ring)] ${
          focused
            ? "bg-(--_dk-ix-bg-selected) shadow-[inset_1px_0_0_var(--_dk-accent-ring)]"
            : ""
        }`}
        {...drag}
        {...dropHandlers}
        drop={dropPosition}
        onContextMenu={onContextMenu}
        onClick={(event) => {
          // A double-click starts renaming; do not re-open on its second click.
          if (!renaming && event.detail < 2) onOpen();
        }}
      >
        <SessionStatusDot status={status}>
          {icon}
          {renaming ? (
            <InlineRename
              key={renameInitialValue}
              initialValue={renameInitialValue}
              ariaLabel={renameAriaLabel}
              maxLength={renameMaxLength}
              allowEmpty={renameAllowEmpty}
              onCommit={(value) => onRename(value)}
              onCancel={onRenameCancel}
              className="w-28 min-w-0 rounded border border-(--_dk-ix-ring) bg-(--_dk-editor) px-1 py-0.5 text-[11px] text-(--_dk-text-secondary) outline-none"
            />
          ) : (
            <span
              className={`max-w-32 min-w-0 ${
                focused
                  ? "text-(--_dk-text-primary)"
                  : "text-(--_dk-text-secondary)"
              }`}
              title={title || label || undefined}
              onDoubleClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                onStartRename?.();
              }}
            >
              <span className="block truncate text-[11px]">
                {label || (
                  <span className="text-(--_dk-text-disabled)">
                    {i18n.t("session.noPreview")}
                  </span>
                )}
              </span>
            </span>
          )}
          <LivePreview
            stepKinds={stepKinds ?? []}
            running={running}
            updatedAt={updatedAt}
            now={now}
          />
        </SessionStatusDot>
      </ListRow>
      <span className="pointer-events-none absolute inset-y-0 right-0 flex items-center gap-1.5 pr-3 text-[10px] text-(--_dk-text-muted) transition-opacity group-hover:opacity-0">
        {unread && (
          <span
            aria-hidden
            className="h-1.5 w-1.5 shrink-0 rounded-full bg-(--_dk-emerald-500)"
          />
        )}
        {relative}
      </span>
      <button
        type="button"
        disabled={deleteBlocked}
        onClick={(event) => {
          event.stopPropagation();
          if (!deleteBlocked) onDelete();
        }}
        aria-label={deleteLabel}
        className="btn-danger btn-icon pointer-events-none absolute right-0 top-0 h-9 w-9 p-0 opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-0 disabled:group-hover:opacity-40"
        title={deleteLabel}
      >
        <Trash size={14} aria-hidden />
      </button>
    </div>
  );
}
