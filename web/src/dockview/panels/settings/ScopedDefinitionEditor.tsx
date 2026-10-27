import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash } from "@phosphor-icons/react";

import type { ToolScope } from "../../../api/settings";
import { FoldCard } from "../../../components/FoldCard";
import { Dropdown, dropdownItemClass } from "../../../components/ui/Dropdown";
import { SettingsPageShell } from "./shared";

/** Scope hint shown by definition pages (MCP). */
export function ScopeHint() {
  const { t } = useTranslation();
  return (
    <p className="text-xs text-(--_dk-text-muted)">
      {t("settings.mcp.bindHint")}
    </p>
  );
}

interface ScopedDefinitionEditorProps<T> {
  title: string;
  /** Label of the "+" button, e.g. "New MCP server". */
  newLabel: string;
  hint: ReactNode;
  global: readonly T[];
  workspace: readonly T[];
  itemId: (item: T) => string;
  /** Muted right-hand summary on each row (command, status …). */
  itemSummary: (item: T) => ReactNode;
  /** Disables Create / New / Delete (save blocked or persist busy). */
  actionsDisabled: boolean;
  isNew: boolean;
  createScope: ToolScope;
  selectedScope: ToolScope;
  selectedId: string | null;
  onCreate: () => void;
  onStartCreate: (scope: ToolScope) => void;
  onCancelNew: () => void;
  onOpen: (scope: ToolScope, id: string) => void;
  onClose: () => void;
  onDelete: (id: string, scope: ToolScope) => void;
  /** Form shown inside the open (or new) definition card. */
  editorForm: ReactNode;
}

/**
 * Global/workspace definition list with a JSON editor card. Used by the MCP
 * settings page; each page owns its own form and persistence while this owns
 * the page shell, scope groups and row chrome.
 */
export function ScopedDefinitionEditor<T>({
  title,
  newLabel,
  hint,
  global,
  workspace,
  itemId,
  itemSummary,
  actionsDisabled,
  isNew,
  createScope,
  selectedScope,
  selectedId,
  onCreate,
  onStartCreate,
  onCancelNew,
  onOpen,
  onClose,
  onDelete,
  editorForm,
}: ScopedDefinitionEditorProps<T>) {
  const { t } = useTranslation();
  return (
    <SettingsPageShell
      title={title}
      actions={
        <>
          {isNew ? (
            <button
              type="button"
              className="btn-primary btn-sm"
              disabled={actionsDisabled}
              onClick={onCreate}
            >
              {t("common.create")}
            </button>
          ) : null}
          <Dropdown
            variant="menu"
            align="right"
            panelClassName="rounded-md"
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className="btn btn-icon"
                disabled={actionsDisabled}
                aria-label={newLabel}
                aria-haspopup="menu"
                aria-expanded={open}
                title={newLabel}
                onClick={toggle}
              >
                <Plus size={16} />
              </button>
            )}
          >
            <button
              type="button"
              className={dropdownItemClass}
              onClick={() => onStartCreate("global")}
            >
              {t("common.global")}
            </button>
            <button
              type="button"
              className={dropdownItemClass}
              onClick={() => onStartCreate("workspace")}
            >
              {t("common.workspace")}
            </button>
          </Dropdown>
        </>
      }
    >
      <div className="space-y-4">
        {hint}
        <div className="space-y-3">
          {(
            [
              ["global", global, t("common.global")],
              ["workspace", workspace, t("common.workspace")],
            ] as const
          ).map(([scope, list, groupTitle]) => (
            <FoldCard
              key={scope}
              defaultOpen
              label={
                <span className="settings-section-title">{groupTitle}</span>
              }
              className="settings-foldcard"
            >
              <div className="space-y-2">
                {list.length === 0 && !(isNew && createScope === scope) ? (
                  <p className="px-2 py-3 text-xs text-(--_dk-text-muted)">
                    {t("common.none")}
                  </p>
                ) : null}
                {list.map((item) => {
                  const id = itemId(item);
                  const selected =
                    !isNew && selectedScope === scope && selectedId === id;
                  return (
                    <FoldCard
                      key={`${scope}:${id}`}
                      open={selected}
                      onToggle={(o) => {
                        if (o) onOpen(scope, id);
                        else if (selectedScope === scope && selectedId === id)
                          onClose();
                      }}
                      label={
                        <span className="flex flex-1 items-center justify-between gap-2">
                          <span className="font-mono text-sm text-(--_dk-text-secondary)">
                            {id}
                          </span>
                          <span className="flex items-center gap-2">
                            <span className="truncate text-xs text-(--_dk-text-muted)">
                              {itemSummary(item)}
                            </span>
                            <button
                              type="button"
                              className="btn-danger btn-icon"
                              disabled={actionsDisabled}
                              onClick={(e) => {
                                e.stopPropagation();
                                onDelete(id, scope);
                              }}
                              onKeyDown={(e) => e.stopPropagation()}
                              aria-label={t("common.deleteNamed", { id })}
                              title={t("common.deleteNamed", { id })}
                            >
                              <Trash size={16} />
                            </button>
                          </span>
                        </span>
                      }
                      className="settings-foldcard"
                    >
                      {selected ? editorForm : null}
                    </FoldCard>
                  );
                })}
                {isNew && createScope === scope ? (
                  <FoldCard
                    key="__new"
                    open
                    onToggle={(o) => {
                      if (!o) onCancelNew();
                    }}
                    label={
                      <span className="font-mono text-sm text-(--_dk-text-secondary)">
                        {t("common.newParen")}
                      </span>
                    }
                    className="settings-foldcard"
                  >
                    {editorForm}
                  </FoldCard>
                ) : null}
              </div>
            </FoldCard>
          ))}
        </div>
      </div>
    </SettingsPageShell>
  );
}
