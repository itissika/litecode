import { useEffect, useRef, useState, type DragEvent, type MouseEvent } from "react";
import { CaretDown, CaretRight, FolderSimple } from "@phosphor-icons/react";

import type { SessionFolder } from "../api/types";
import i18n from "../i18n";

interface SessionFolderHeaderProps {
  folder: SessionFolder;
  count: number;
  collapsed: boolean;
  renaming: boolean;
  dragOver: boolean;
  onToggle: () => void;
  onContextMenu: (event: MouseEvent) => void;
  onRename: (name: string) => void;
  onRenameCancel: () => void;
  onDragOver: (event: DragEvent) => void;
  onDragLeave: (event: DragEvent) => void;
  onDrop: (event: DragEvent) => void;
}

/** Folder row: collapse toggle, inline rename, and a drop target for sessions. */
export function SessionFolderHeader({
  folder,
  count,
  collapsed,
  renaming,
  dragOver,
  onToggle,
  onContextMenu,
  onRename,
  onRenameCancel,
  onDragOver,
  onDragLeave,
  onDrop,
}: SessionFolderHeaderProps) {
  const [draft, setDraft] = useState(folder.name);
  const inputRef = useRef<HTMLInputElement>(null);
  const doneRef = useRef(false);

  useEffect(() => {
    if (!renaming) return;
    doneRef.current = false;
    setDraft(folder.name);
    const raf = requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => cancelAnimationFrame(raf);
  }, [renaming, folder.name]);

  const finish = (value: string) => {
    if (doneRef.current) return;
    doneRef.current = true;
    const name = value.trim();
    if (name && name !== folder.name) onRename(name);
    else onRenameCancel();
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-expanded={!collapsed}
      aria-label={`${
        collapsed
          ? i18n.t("session.expandFolder")
          : i18n.t("session.collapseFolder")
      }: ${folder.name}`}
      onClick={onToggle}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onToggle();
        }
      }}
      onContextMenu={onContextMenu}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      className={`group flex h-8 cursor-pointer items-center gap-1.5 rounded px-2 text-[11px] text-(--_dk-text-secondary) transition-colors ${
        dragOver
          ? "bg-(--_dk-ix-bg-selected) shadow-[0_0_0_1px_var(--_dk-ix-ring)]"
          : "hover:bg-(--_dk-ix-bg-hover)"
      }`}
    >
      {collapsed ? (
        <CaretRight size={10} className="shrink-0 text-(--_dk-text-muted)" />
      ) : (
        <CaretDown size={10} className="shrink-0 text-(--_dk-text-muted)" />
      )}
      <FolderSimple size={13} className="shrink-0 text-(--_dk-text-muted)" />
      {renaming ? (
        <input
          ref={inputRef}
          value={draft}
          onClick={(event) => event.stopPropagation()}
          onMouseDown={(event) => event.stopPropagation()}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            // The header treats Enter/Space as collapse; typing in the rename
            // input must not also toggle the folder.
            event.stopPropagation();
            if (event.key === "Enter") {
              event.preventDefault();
              finish(draft);
            } else if (event.key === "Escape") {
              event.preventDefault();
              doneRef.current = true;
              onRenameCancel();
            }
          }}
          onBlur={() => finish(draft)}
          aria-label={i18n.t("session.folderName")}
          maxLength={120}
          className="min-w-0 flex-1 rounded border border-(--_dk-ix-ring) bg-(--_dk-editor) px-1 py-0.5 text-[11px] text-(--_dk-text-secondary) outline-none"
        />
      ) : (
        <span className="min-w-0 flex-1 truncate">{folder.name}</span>
      )}
      <span className="ml-auto shrink-0 text-[10px] text-(--_dk-text-disabled)">
        {count}
      </span>
    </div>
  );
}
