import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";

import type { BotBody, BotInfo } from "../api/types";
import {
  applyToolEnabled,
  getAgent,
  getAvailableTools,
  getMcpServers,
  presetToolIds,
  settingsToolSwitches,
  type AgentProfile,
  type AgentToolBinding,
  type AvailableTool,
  type McpServerItem,
} from "../api/settings";
import { useSessionStore } from "../stores/sessionStore";
import { useBotStore } from "../stores/botStore";
import { usePortalTarget } from "./ui/usePortalTarget";
import { Select } from "./ui/Select";
import { AgentProfileFields } from "./agent/AgentProfileFields";
import { AgentToolsGrid, bindingFor } from "./agent/AgentToolsGrid";
import { FieldLabel, TextInput } from "./ui/FormField";
import i18n from "../i18n";

export type BotDialogMode =
  | { kind: "create" }
  | { kind: "edit"; bot: BotInfo };

interface BotDialogProps {
  mode: BotDialogMode;
  onClose: () => void;
  onCreated?: (bot: BotInfo) => void;
}

function draftFromBody(body: BotBody): AgentProfile {
  return {
    role: "primary",
    model_ref: "",
    system_prompt: body.system_prompt,
    description: "",
    tools: body.tools,
    allowed_subagents: body.allowed_subagents,
  };
}

function bodyFromDraft(draft: AgentProfile): BotBody {
  return {
    system_prompt: draft.system_prompt,
    tools: draft.tools,
    allowed_subagents: draft.allowed_subagents,
  };
}

function isBusy(bot: BotInfo, sessions: ReturnType<typeof useSessionStore.getState>["sessions"]) {
  const session = sessions.find((item) => item.id === bot.session_id);
  return Boolean(
    session?.running ||
      session?.status === "running_with_subagent" ||
      session?.status === "stopping",
  );
}

/** Create/edit form for a bot identity and its owned capability profile. */
export function BotDialog({ mode, onClose, onCreated }: BotDialogProps) {
  const titleId = useId();
  const nameId = useId();
  const descriptionId = useId();
  const templateFieldId = useId();
  const { anchorRef, target } = usePortalTarget<HTMLSpanElement>();
  const panelRef = useRef<HTMLDivElement>(null);
  const primaryAgents = useSessionStore((s) => s.primaryAgents);
  const activePrimary = useSessionStore((s) => s.activePrimary);
  const sessions = useSessionStore((s) => s.sessions);
  const createBot = useBotStore((s) => s.createBot);
  const updateBot = useBotStore((s) => s.updateBot);
  const editing = mode.kind === "edit" ? mode.bot : null;
  const [name, setName] = useState(editing?.name ?? "");
  const [description, setDescription] = useState(editing?.description ?? "");
  const [templateId, setTemplateId] = useState(activePrimary);
  const [profile, setProfile] = useState<AgentProfile | null>(
    editing ? draftFromBody(editing.body) : null,
  );
  const [tools, setTools] = useState<AvailableTool[]>([]);
  const [mcpServers, setMcpServers] = useState<McpServerItem[]>([]);
  const [toolsLoading, setToolsLoading] = useState(true);
  const [templateLoading, setTemplateLoading] = useState(!editing);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const busy = editing ? isBusy(editing, sessions) : false;
  const agents = useMemo(() => {
    if (primaryAgents.length > 0) return primaryAgents;
    return activePrimary ? [{ id: activePrimary, description: "" }] : [];
  }, [primaryAgents, activePrimary]);
  const selectedTemplate = agents.some((agent) => agent.id === templateId)
    ? templateId
    : (agents[0]?.id ?? "");
  const bindableTools = useMemo(() => settingsToolSwitches(tools), [tools]);
  const presetIds = useMemo(() => presetToolIds(bindableTools), [bindableTools]);

  useEffect(() => {
    let cancelled = false;
    setToolsLoading(true);
    Promise.all([
      getAvailableTools(),
      // MCP discovery only feeds the per-server tool picker. A failure here
      // must not hide the rest of the body editor.
      getMcpServers().catch(() => null),
    ])
      .then(([loadedTools, loadedMcp]) => {
        if (cancelled) return;
        setTools(loadedTools);
        setMcpServers(
          loadedMcp ? [...loadedMcp.global, ...loadedMcp.workspace] : [],
        );
        setToolsLoading(false);
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setToolsLoading(false);
        setError(
          reason instanceof Error
            ? reason.message
            : i18n.t("bot.profileLoadFailed"),
        );
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // A new bot starts from the template's body; the record, not the template
  // profile, is what gets persisted.
  useEffect(() => {
    if (editing || !selectedTemplate) return;
    let cancelled = false;
    setTemplateLoading(true);
    setProfile(null);
    setError(null);
    getAgent(selectedTemplate)
      .then((template) => {
        if (cancelled) return;
        setProfile(
          draftFromBody({
            system_prompt: template.system_prompt,
            tools: template.tools,
            allowed_subagents: template.allowed_subagents,
          }),
        );
        setTemplateLoading(false);
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setTemplateLoading(false);
        setProfile(null);
        setError(
          reason instanceof Error
            ? reason.message
            : i18n.t("bot.profileLoadFailed"),
        );
      });
    return () => {
      cancelled = true;
    };
  }, [editing, selectedTemplate]);

  useEffect(() => {
    const panel = panelRef.current;
    if (!target || !panel) return;
    panel.querySelector<HTMLElement>("input, textarea, select")?.focus();
  }, [target, toolsLoading, templateLoading]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    const panel = panelRef.current;
    if (!panel) return;
    const focusable = [...panel.querySelectorAll<HTMLElement>(
      "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled])",
    )];
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = panel.ownerDocument.activeElement;
    if (event.shiftKey && active === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    // The dialog can be mounted inside another form (the chat composer). A
    // portal keeps the DOM outside it, but React events still bubble through
    // the owner tree, so stop the submit before it reaches the composer.
    event.stopPropagation();
    const trimmed = name.trim();
    if (!trimmed || saving) return;
    setSaving(true);
    setError(null);
    if (!profile) {
      setSaving(false);
      setError(i18n.t("bot.profileLoadFailed"));
      return;
    }
    const body = bodyFromDraft(profile);
    if (!editing) {
      if (!selectedTemplate) {
        setSaving(false);
        setError(i18n.t("bot.noAgents"));
        return;
      }
      const bot = await createBot(trimmed, description.trim(), body);
      setSaving(false);
      if (!bot) {
        setError(i18n.t("bot.createFailed"));
        return;
      }
      onCreated?.(bot);
      onClose();
      return;
    }
    // While a bot is running, identity edits are safe; body edits are not
    // applied silently. The server enforces the same split.
    const updated = await updateBot(editing.id, {
      name: trimmed,
      description: description.trim(),
      ...(busy ? {} : { body }),
    });
    setSaving(false);
    if (!updated) {
      setError(i18n.t("bot.updateFailed"));
      return;
    }
    onClose();
  };

  const updateBinding = (toolId: string, patch: Partial<AgentToolBinding>) => {
    if (!profile) return;
    if (patch.enabled !== undefined) {
      setProfile({
        ...profile,
        tools: applyToolEnabled(
          profile.tools,
          toolId,
          patch.enabled,
          presetIds,
        ),
      });
      return;
    }
    const current = bindingFor(profile.tools, toolId);
    setProfile({
      ...profile,
      tools: {
        ...profile.tools,
        [toolId]: { ...current, ...patch },
      },
    });
  };

  if (!target) {
    return <span ref={anchorRef} hidden aria-hidden />;
  }

  return (
    <>
      <span ref={anchorRef} hidden aria-hidden />
      {createPortal(
        <div
          data-litecode-overlay="bot-dialog"
          className="fixed inset-0 z-(--_dk-z-dialog) flex items-center justify-center bg-(--_dk-root)/60 p-4"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) onClose();
          }}
        >
          <div
            ref={panelRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            onKeyDown={onKeyDown}
            className="flex max-h-[85vh] w-full max-w-3xl flex-col rounded-lg border border-(--_dk-line-visible) bg-(--_dk-overlay) text-(--_dk-text-secondary) shadow-(--_dk-elevation)"
          >
            <div className="flex items-center justify-between border-b border-(--_dk-line) px-6 py-3.5">
              <h2 id={titleId} className="settings-section-title">
                {editing ? i18n.t("bot.editTitle") : i18n.t("bot.createTitle")}
              </h2>
              <button
                type="button"
                onClick={onClose}
                className="btn-ghost btn-sm"
                aria-label={i18n.t("common.close")}
              >
                {i18n.t("common.close")}
              </button>
            </div>

            <form onSubmit={submit} className="flex min-h-0 flex-1 flex-col">
              <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-6 py-5">
                {error && (
                  <p className="rounded-md border border-(--_dk-ix-danger-border) bg-(--_dk-ix-danger-bg) px-3 py-2 text-xs text-(--_dk-ix-danger-fg)">
                    {error}
                  </p>
                )}

                <section className="space-y-2">
                  <h3 className="settings-section-title">
                    {i18n.t("bot.identity")}
                  </h3>
                  <div className="settings-card space-y-3 p-3">
                    <div>
                      <FieldLabel htmlFor={nameId}>
                        {i18n.t("bot.name")}
                      </FieldLabel>
                      <TextInput
                        id={nameId}
                        value={name}
                        maxLength={48}
                        onChange={(event) => setName(event.target.value)}
                        placeholder={i18n.t("bot.namePlaceholder")}
                      />
                    </div>
                    <div>
                      <FieldLabel htmlFor={descriptionId}>
                        {i18n.t("bot.description")}
                      </FieldLabel>
                      <TextInput
                        id={descriptionId}
                        value={description}
                        maxLength={280}
                        onChange={(event) => setDescription(event.target.value)}
                        placeholder={i18n.t("bot.descriptionPlaceholder")}
                      />
                    </div>
                    {!editing && (
                      <div>
                        <FieldLabel htmlFor={templateFieldId}>
                          {i18n.t("bot.template")}
                        </FieldLabel>
                        <Select
                          id={templateFieldId}
                          value={selectedTemplate}
                          onChange={setTemplateId}
                          placeholder={i18n.t("bot.noAgents")}
                          className="w-full"
                          options={agents.map((agent) => ({
                            value: agent.id,
                            label: agent.description
                              ? `${agent.id} — ${agent.description}`
                              : agent.id,
                          }))}
                        />
                        <p className="settings-field-hint">
                          {i18n.t("bot.inheritHint")}
                        </p>
                      </div>
                    )}
                    {editing && (
                      <div>
                        <FieldLabel>{i18n.t("bot.type")}</FieldLabel>
                        <div className="flex items-center justify-between rounded-md border border-(--_dk-line) px-3 py-2 text-xs">
                          <span className="text-(--_dk-text-primary)">
                            {i18n.t("bot.typeBot")}
                          </span>
                          <span className="text-[11px] text-(--_dk-text-disabled)">
                            {i18n.t("bot.typeHint")}
                          </span>
                        </div>
                      </div>
                    )}
                  </div>
                </section>

                <section className="space-y-3">
                  <h3 className="settings-section-title">
                    {i18n.t("bot.capability")}
                  </h3>
                  {busy && (
                    <p className="rounded-md border border-(--_dk-amber-500)/40 bg-(--_dk-amber-500)/10 px-3 py-2 text-xs text-(--_dk-amber-500)">
                      {i18n.t("bot.busyCannotEdit")}
                    </p>
                  )}
                  {templateLoading ? (
                    <p className="text-xs text-(--_dk-text-disabled)">
                      {i18n.t("bot.loadingProfile")}
                    </p>
                  ) : profile ? (
                    <>
                      <AgentProfileFields
                        draft={profile}
                        saveBlocked={busy}
                        onChange={setProfile}
                        showDescription={false}
                      />
                      <AgentToolsGrid
                        draft={profile}
                        bindableTools={bindableTools}
                        mcpServers={mcpServers}
                        saveBlocked={busy || toolsLoading}
                        onBindingChange={updateBinding}
                        gridStyle={{
                          gridTemplateColumns:
                            "repeat(auto-fill, minmax(220px, 1fr))",
                        }}
                      />
                    </>
                  ) : (
                    <p className="text-xs text-(--_dk-ix-danger-fg)">
                      {i18n.t("bot.profileLoadFailed")}
                    </p>
                  )}
                </section>
              </div>

              <div className="flex justify-end gap-2 border-t border-(--_dk-line) px-6 py-3">
                <button
                  type="button"
                  className="btn btn-sm btn-ghost"
                  onClick={onClose}
                >
                  {i18n.t("common.cancel")}
                </button>
                <button
                  type="submit"
                  className="btn btn-sm btn-primary"
                  disabled={
                    saving ||
                    !name.trim() ||
                    !profile ||
                    (!editing && !selectedTemplate)
                  }
                >
                  {saving
                    ? i18n.t("bot.saving")
                    : editing
                      ? i18n.t("common.save")
                      : i18n.t("bot.create")}
                </button>
              </div>
            </form>
          </div>
        </div>,
        target,
      )}
    </>
  );
}