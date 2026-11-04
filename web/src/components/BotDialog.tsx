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

import type { BotInfo } from "../api/types";
import {
  getAgent,
  getAvailableTools,
  settingsToolSwitches,
  type AgentProfile,
  type AvailableTool,
  type ToolPreset,
} from "../api/settings";
import { useSessionStore } from "../stores/sessionStore";
import { useBotStore } from "../stores/botStore";
import { usePortalTarget } from "./ui/usePortalTarget";
import i18n from "../i18n";

export type BotDialogMode =
  | { kind: "create" }
  | { kind: "edit"; bot: BotInfo };

interface BotDialogProps {
  mode: BotDialogMode;
  onClose: () => void;
  onCreated?: (bot: BotInfo) => void;
}

function isBusy(bot: BotInfo, sessions: ReturnType<typeof useSessionStore.getState>["sessions"]) {
  const session = sessions.find((item) => item.id === bot.default_session_id);
  return Boolean(
    session?.running ||
      session?.status === "running_with_subagent" ||
      session?.status === "stopping",
  );
}

/** Create/edit form for a bot identity and its owned capability profile. */
export function BotDialog({ mode, onClose, onCreated }: BotDialogProps) {
  const titleId = useId();
  const { anchorRef, target } = usePortalTarget<HTMLSpanElement>();
  const panelRef = useRef<HTMLDivElement>(null);
  const primaryAgents = useSessionStore((s) => s.primaryAgents);
  const activePrimary = useSessionStore((s) => s.activePrimary);
  const availableModels = useSessionStore((s) => s.availableModels);
  const sessions = useSessionStore((s) => s.sessions);
  const createBot = useBotStore((s) => s.createBot);
  const updateBot = useBotStore((s) => s.updateBot);
  const editing = mode.kind === "edit" ? mode.bot : null;
  const [name, setName] = useState(editing?.name ?? "");
  const [description, setDescription] = useState(editing?.description ?? "");
  const [templateId, setTemplateId] = useState(activePrimary);
  const [profile, setProfile] = useState<AgentProfile | null>(null);
  const [tools, setTools] = useState<AvailableTool[]>([]);
  const [loading, setLoading] = useState(mode.kind === "edit");
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

  useEffect(() => {
    if (!editing) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    Promise.all([getAgent(editing.profile_id), getAvailableTools()])
      .then(([loadedProfile, loadedTools]) => {
        if (cancelled) return;
        setProfile(loadedProfile);
        setTools(loadedTools);
        setLoading(false);
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setLoading(false);
        setError(
          reason instanceof Error
            ? reason.message
            : i18n.t("bot.profileLoadFailed"),
        );
      });
    return () => {
      cancelled = true;
    };
  }, [editing?.profile_id]);

  useEffect(() => {
    const panel = panelRef.current;
    if (!target || !panel) return;
    const first = panel.querySelector<HTMLElement>("[data-bot-autofocus]");
    first?.focus();
  }, [target, loading]);

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
    const trimmed = name.trim();
    if (!trimmed || saving) return;
    setSaving(true);
    setError(null);
    if (!editing) {
      if (!selectedTemplate) {
        setSaving(false);
        setError(i18n.t("bot.noAgents"));
        return;
      }
      const bot = await createBot(trimmed, description.trim(), selectedTemplate);
      setSaving(false);
      if (!bot) {
        setError(i18n.t("bot.createFailed"));
        return;
      }
      onCreated?.(bot);
      onClose();
      return;
    }
    if (!profile && !busy) {
      setSaving(false);
      setError(i18n.t("bot.profileLoadFailed"));
      return;
    }
    // While a bot is running, identity edits are safe; capability edits are not
    // applied silently. The server enforces the same split.
    const updated = await updateBot(editing.id, {
      name: trimmed,
      description: description.trim(),
      ...(busy || !profile ? {} : { profile }),
    });
    setSaving(false);
    if (!updated) {
      setError(i18n.t("bot.updateFailed"));
      return;
    }
    onClose();
  };

  const toggleTool = (tool: AvailableTool, enabled: boolean) => {
    if (!profile) return;
    const current = profile.tools[tool.id];
    setProfile({
      ...profile,
      tools: {
        ...profile.tools,
        [tool.id]: { ...current, enabled },
      },
    });
  };

  const setToolPreset = (tool: AvailableTool, preset: ToolPreset | null) => {
    if (!profile) return;
    const current = profile.tools[tool.id];
    setProfile({
      ...profile,
      tools: {
        ...profile.tools,
        [tool.id]: { ...current, enabled: true, preset },
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
            className="flex max-h-[85vh] w-full max-w-2xl flex-col rounded-lg border border-(--_dk-line-visible) bg-(--_dk-overlay) text-(--_dk-text-secondary) shadow-(--_dk-elevation)"
          >
            <div className="flex items-center justify-between border-b border-(--_dk-line) px-4 py-3">
              <h2 id={titleId} className="text-sm font-medium text-(--_dk-text-primary)">
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
              <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-4 py-4">
                {error && (
                  <p className="rounded-md border border-(--_dk-ix-danger-border) bg-(--_dk-ix-danger-bg) px-3 py-2 text-xs text-(--_dk-ix-danger-fg)">
                    {error}
                  </p>
                )}

                <section className="space-y-2">
                  <h3 className="settings-section-title">{i18n.t("bot.identity")}</h3>
                  <label className="block space-y-1">
                    <span className="text-[11px] text-(--_dk-text-muted)">
                      {i18n.t("bot.name")}
                    </span>
                    <input
                      data-bot-autofocus
                      value={name}
                      maxLength={48}
                      onChange={(event) => setName(event.target.value)}
                      placeholder={i18n.t("bot.namePlaceholder")}
                      className="w-full rounded-md border border-(--_dk-line) bg-(--_dk-editor) px-2 py-1.5 text-xs text-(--_dk-text-primary) outline-none focus:border-(--_dk-accent-hover)"
                    />
                  </label>
                  <label className="block space-y-1">
                    <span className="text-[11px] text-(--_dk-text-muted)">
                      {i18n.t("bot.description")}
                    </span>
                    <textarea
                      value={description}
                      maxLength={280}
                      rows={2}
                      onChange={(event) => setDescription(event.target.value)}
                      placeholder={i18n.t("bot.descriptionPlaceholder")}
                      className="w-full rounded-md border border-(--_dk-line) bg-(--_dk-editor) px-2 py-1.5 text-xs text-(--_dk-text-primary) outline-none focus:border-(--_dk-accent-hover)"
                    />
                  </label>
                  {!editing && (
                    <label className="block space-y-1">
                      <span className="text-[11px] text-(--_dk-text-muted)">
                        {i18n.t("bot.template")}
                      </span>
                      <select
                        value={selectedTemplate}
                        onChange={(event) => setTemplateId(event.target.value)}
                        className="w-full rounded-md border border-(--_dk-line) bg-(--_dk-side) px-2 py-1.5 text-xs text-(--_dk-text-primary) outline-none focus:border-(--_dk-accent-hover)"
                      >
                        {agents.map((agent) => (
                          <option key={agent.id} value={agent.id}>
                            {agent.id}
                            {agent.description ? ` — ${agent.description}` : ""}
                          </option>
                        ))}
                      </select>
                      <p className="text-[11px] text-(--_dk-text-muted)">
                        {i18n.t("bot.inheritHint")}
                      </p>
                    </label>
                  )}
                  {editing && (
                    <div className="flex items-center justify-between rounded-md border border-(--_dk-line) px-3 py-2 text-xs">
                      <div>
                        <span className="text-(--_dk-text-muted)">
                          {i18n.t("bot.type")}
                        </span>
                        <span className="ml-2 text-(--_dk-text-primary)">
                          {i18n.t("bot.typeBot")}
                        </span>
                      </div>
                      <span className="text-[11px] text-(--_dk-text-disabled)">
                        {i18n.t("bot.typeHint")}
                      </span>
                    </div>
                  )}
                </section>

                {editing && (
                  <section className="space-y-3">
                    <h3 className="settings-section-title">
                      {i18n.t("bot.capability")}
                    </h3>
                    {busy && (
                      <p className="rounded-md border border-(--_dk-amber-500)/40 bg-(--_dk-amber-500)/10 px-3 py-2 text-xs text-(--_dk-amber-500)">
                        {i18n.t("bot.busyCannotEdit")}
                      </p>
                    )}
                    {loading ? (
                      <p className="text-xs text-(--_dk-text-disabled)">
                        {i18n.t("bot.loadingProfile")}
                      </p>
                    ) : profile ? (
                      <>
                        <label className="block space-y-1">
                          <span className="text-[11px] text-(--_dk-text-muted)">
                            {i18n.t("bot.model")}
                          </span>
                          <select
                            value={profile.model_ref}
                            disabled={busy}
                            onChange={(event) =>
                              setProfile({ ...profile, model_ref: event.target.value })
                            }
                            className="w-full rounded-md border border-(--_dk-line) bg-(--_dk-side) px-2 py-1.5 text-xs text-(--_dk-text-primary) outline-none focus:border-(--_dk-accent-hover)"
                          >
                            {!availableModels.some(
                              (model) => model.id === profile.model_ref,
                            ) && profile.model_ref ? (
                              <option value={profile.model_ref}>
                                {profile.model_ref}
                              </option>
                            ) : null}
                            {availableModels.map((model) => (
                              <option key={model.id} value={model.id}>
                                {model.label || model.id}
                              </option>
                            ))}
                          </select>
                        </label>
                        <label className="block space-y-1">
                          <span className="text-[11px] text-(--_dk-text-muted)">
                            {i18n.t("bot.systemPrompt")}
                          </span>
                          <textarea
                            value={profile.system_prompt}
                            rows={4}
                            disabled={busy}
                            onChange={(event) =>
                              setProfile({
                                ...profile,
                                system_prompt: event.target.value,
                              })
                            }
                            className="w-full rounded-md border border-(--_dk-line) bg-(--_dk-editor) px-2 py-1.5 text-xs text-(--_dk-text-primary) outline-none focus:border-(--_dk-accent-hover)"
                          />
                        </label>
                        <label className="block space-y-1">
                          <span className="text-[11px] text-(--_dk-text-muted)">
                            {i18n.t("bot.maxSteps")}
                          </span>
                          <input
                            type="number"
                            min={1}
                            value={profile.max_steps}
                            disabled={busy}
                            onChange={(event) =>
                              setProfile({
                                ...profile,
                                max_steps: Number(event.target.value) || 1,
                              })
                            }
                            className="w-full rounded-md border border-(--_dk-line) bg-(--_dk-editor) px-2 py-1.5 text-xs text-(--_dk-text-primary) outline-none focus:border-(--_dk-accent-hover)"
                          />
                        </label>
                        <div className="space-y-1">
                          <span className="text-[11px] text-(--_dk-text-muted)">
                            {i18n.t("bot.tools")}
                          </span>
                          <div className="max-h-56 space-y-1 overflow-y-auto rounded-md border border-(--_dk-line) p-2">
                            {bindableTools.length === 0 ? (
                              <p className="text-[11px] text-(--_dk-text-disabled)">
                                {i18n.t("bot.noTools")}
                              </p>
                            ) : (
                              bindableTools.map((tool) => {
                                const binding = profile.tools[tool.id];
                                return (
                                  <div
                                    key={tool.id}
                                    className="flex items-center gap-2 rounded px-1 py-1 text-xs hover:bg-(--_dk-ix-bg-hover)"
                                  >
                                    <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-2">
                                      <input
                                        type="checkbox"
                                        checked={binding?.enabled === true}
                                        disabled={busy}
                                        onChange={(event) =>
                                          toggleTool(tool, event.target.checked)
                                        }
                                        className="accent-(--_dk-accent-hover)"
                                      />
                                      <span className="truncate font-mono text-[11px]">
                                        {tool.label || tool.id}
                                      </span>
                                    </label>
                                    {tool.permission_surface === "preset" &&
                                      binding?.enabled && (
                                        <select
                                          value={binding.preset ?? ""}
                                          disabled={busy}
                                          onChange={(event) =>
                                            setToolPreset(
                                              tool,
                                              event.target.value
                                                ? (event.target.value as ToolPreset)
                                                : null,
                                            )
                                          }
                                          className="rounded border border-(--_dk-line) bg-(--_dk-side) px-1 py-0.5 text-[10px]"
                                        >
                                          <option value="">
                                            {i18n.t("bot.toolDefault")}
                                          </option>
                                          <option value="ALL">
                                            {i18n.t("bot.toolAll")}
                                          </option>
                                          <option value="SAFE">
                                            {i18n.t("bot.toolSafe")}
                                          </option>
                                        </select>
                                      )}
                                  </div>
                                );
                              })
                            )}
                          </div>
                        </div>
                      </>
                    ) : (
                      <p className="text-xs text-(--_dk-ix-danger-fg)">
                        {i18n.t("bot.profileLoadFailed")}
                      </p>
                    )}
                  </section>
                )}
              </div>

              <div className="flex justify-end gap-2 border-t border-(--_dk-line) px-4 py-3">
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
                    (!editing && !selectedTemplate) ||
                    (editing !== null && loading)
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
