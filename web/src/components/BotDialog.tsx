import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
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
import { useConnectionStore } from "../stores/connectionStore";
import { useToastStore } from "../stores/toastStore";
import { usePortalTarget } from "./ui/usePortalTarget";
import { AgentProfileFields } from "./agent/AgentProfileFields";
import { AgentToolsGrid, bindingFor } from "./agent/AgentToolsGrid";
import { FieldLabel, TextInput } from "./ui/FormField";
import { ModelSwitcher } from "./ModelSwitcher";
import i18n from "../i18n";

export type BotDialogMode =
  | { kind: "create" }
  | { kind: "edit"; bot: BotInfo };

interface BotDialogProps {
  mode: BotDialogMode;
  onClose: () => void;
  onCreated?: (bot: BotInfo) => void;
}

/**
 * Every bot is built on the locked built-in `default` spec (D2): the bot record
 * owns only its role text and its non-core tool overrides.
 */
export const BOT_BASE_AGENT_ID = "default";

/**
 * `default`'s optional builtins that the bot base keeps on no matter what the
 * Settings → Agents page last saved (`bot::resolve::spec_tool_bindings`).
 */
export const BOT_DEFAULT_OPTIONAL_TOOL_IDS = [
  "code_search",
  "webfetch",
  "websearch",
] as const;

/** Widest the sheet ever gets; matches the panel's `max-w-[70rem]`. */
const DIALOG_MAX_WIDTH = 1120;
const DIALOG_MIN_WIDTH = 520;
const DIALOG_MIN_HEIGHT = 360;
/** Overlay padding (`p-4`) kept clear on each side. */
const DIALOG_VIEWPORT_GUTTER = 32;

/**
 * Sheet size after dragging the corner grip by `(dx, dy)`.
 *
 * The sheet stays centered over the viewport, so growing it by `2d` moves the
 * bottom-right corner by `d` — the corner then tracks the pointer exactly. The
 * caps mirror the panel classes (`max-h-[85vh]`, `max-w-[70rem]`), so a drag
 * cannot push the sheet past its resting bounds.
 */
export function botDialogSize(
  start: { w: number; h: number },
  dx: number,
  dy: number,
  viewport: { w: number; h: number },
): { w: number; h: number } {
  const clamp = (value: number, min: number, max: number) =>
    Math.min(Math.max(value, min), Math.max(min, max));
  return {
    w: clamp(
      start.w + dx * 2,
      DIALOG_MIN_WIDTH,
      Math.min(DIALOG_MAX_WIDTH, viewport.w - DIALOG_VIEWPORT_GUTTER),
    ),
    h: clamp(start.h + dy * 2, DIALOG_MIN_HEIGHT, viewport.h * 0.85),
  };
}

function lockedToolIds(availableTools: readonly AvailableTool[]): Set<string> {
  return new Set(
    availableTools.filter((tool) => tool.locked).map((tool) => tool.id),
  );
}

/**
 * Build the capability grid draft for a bot (D2/D3/I3).
 *
 * The backend only stores the user's non-core overrides, so the grid is rebuilt
 * on every load: locked core cards mirror the `default` spec (they are read-only
 * in the grid), the spec's optional defaults start on, and the saved record's
 * non-core entries are layered on top.
 */
export function composeBotDraftTools(
  defaultTools: Record<string, AgentToolBinding>,
  availableTools: readonly AvailableTool[],
  savedBodyTools: Record<string, AgentToolBinding>,
): Record<string, AgentToolBinding> {
  const locked = lockedToolIds(availableTools);
  const draft: Record<string, AgentToolBinding> = {};
  for (const id of locked) {
    // A locked card always reports the spec's binding.
    draft[id] = defaultTools[id] ?? { enabled: true, preset: null };
  }
  for (const id of BOT_DEFAULT_OPTIONAL_TOOL_IDS) {
    draft[id] = { enabled: true, preset: null };
  }
  for (const [id, binding] of Object.entries(savedBodyTools)) {
    // The record never owns a core key; a stale one must not re-open a lock.
    if (locked.has(id)) continue;
    draft[id] = { ...binding };
  }
  return draft;
}

/** Keep only the bindings a bot record may own: the non-core overrides (I3). */
export function botToolOverrides(
  tools: Record<string, AgentToolBinding>,
  availableTools: readonly AvailableTool[],
): Record<string, AgentToolBinding> {
  const locked = lockedToolIds(availableTools);
  const overrides: Record<string, AgentToolBinding> = {};
  for (const [id, binding] of Object.entries(tools)) {
    if (locked.has(id)) continue;
    overrides[id] = binding;
  }
  return overrides;
}

function bodyFromDraft(
  draft: AgentProfile,
  availableTools: readonly AvailableTool[],
): BotBody {
  return {
    system_prompt: draft.system_prompt,
    tools: botToolOverrides(draft.tools, availableTools),
    // Fixed by the `default` spec (`["explore"]`); the record cannot move it.
    allowed_subagents: [],
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

/** Create/edit form for a bot identity and its role text. */
export function BotDialog({ mode, onClose, onCreated }: BotDialogProps) {
  const titleId = useId();
  const nameId = useId();
  const descriptionId = useId();
  const { anchorRef, target } = usePortalTarget<HTMLSpanElement>();
  const panelRef = useRef<HTMLDivElement>(null);
  const sessions = useSessionStore((s) => s.sessions);
  const createBot = useBotStore((s) => s.createBot);
  const updateBot = useBotStore((s) => s.updateBot);
  const editing = mode.kind === "edit" ? mode.bot : null;
  const [name, setName] = useState(editing?.name ?? "");
  const [description, setDescription] = useState(editing?.description ?? "");
  const [profile, setProfile] = useState<AgentProfile | null>(null);
  const [tools, setTools] = useState<AvailableTool[]>([]);
  const [mcpServers, setMcpServers] = useState<McpServerItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Sheet size while the user drags the corner grip. Session-only — nothing is
  // persisted — and a double-click on the grip returns to the content-fit size.
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
  const resizeStart = useRef<{
    x: number;
    y: number;
    w: number;
    h: number;
  } | null>(null);

  const busy = editing ? isBusy(editing, sessions) : false;
  // The bot record has no model field: the session owns it. Read the listed
  // row first and prefer the live snapshot once that session is open.
  const listedSessionModel = editing
    ? (sessions.find((item) => item.id === editing.session_id)?.model_id ?? null)
    : null;
  const liveSessionModel = useSessionStore((s) =>
    editing ? (s.byId.get(editing.session_id)?.modelId ?? null) : null,
  );
  const sessionModel = liveSessionModel ?? listedSessionModel;
  // Create has no session yet, so the picker lives in a local draft. Edit uses
  // the same draft but starts empty ("follow the session") and only sends a
  // change on Save, keeping Cancel side-effect free.
  const [modelDraft, setModelDraft] = useState<string | undefined>();
  const modelId = modelDraft ?? sessionModel;
  const modelChanged = modelDraft !== undefined && modelDraft !== sessionModel;
  const bindableTools = useMemo(() => settingsToolSwitches(tools), [tools]);
  const presetIds = useMemo(() => presetToolIds(bindableTools), [bindableTools]);

  // Create and edit load the same base: the tool catalog, the locked `default`
  // spec (the only base a bot can have), and the MCP catalog for the per-server
  // picker. An MCP failure must not hide the rest of the body editor.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    Promise.all([
      getAvailableTools(),
      getAgent(BOT_BASE_AGENT_ID),
      getMcpServers().catch(() => null),
    ])
      .then(([loadedTools, base, loadedMcp]) => {
        if (cancelled) return;
        setTools(loadedTools);
        setMcpServers(
          loadedMcp ? [...loadedMcp.global, ...loadedMcp.workspace] : [],
        );
        setProfile({
          role: "primary",
          model_ref: "",
          // New bots start empty: the base prompt is never copied into the
          // role box, it is appended by the backend as `# Bot Role`.
          system_prompt: editing?.body.system_prompt ?? "",
          description: "",
          tools: composeBotDraftTools(
            base.tools,
            loadedTools,
            editing?.body.tools ?? {},
          ),
          allowed_subagents: [],
        });
        setLoading(false);
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setProfile(null);
        setLoading(false);
        setError(
          // Always name the failure; the transport detail rides along.
          reason instanceof Error
            ? `${i18n.t("bot.profileLoadFailed")} ${reason.message}`.trim()
            : i18n.t("bot.profileLoadFailed"),
        );
      });
    return () => {
      cancelled = true;
    };
  }, [editing]);

  useEffect(() => {
    const panel = panelRef.current;
    if (!target || !panel) return;
    panel.querySelector<HTMLElement>("input, textarea, select")?.focus();
  }, [target, loading]);

  const onResizeStart = (event: ReactPointerEvent<HTMLDivElement>) => {
    const panel = panelRef.current;
    if (!panel) return;
    event.preventDefault();
    // Start from the rendered box so the sheet never jumps on the first move.
    const rect = panel.getBoundingClientRect();
    resizeStart.current = {
      x: event.clientX,
      y: event.clientY,
      w: rect.width,
      h: rect.height,
    };
    setSize({ w: rect.width, h: rect.height });
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };

  const onResizeMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const start = resizeStart.current;
    if (!start) return;
    setSize(
      botDialogSize(
        start,
        event.clientX - start.x,
        event.clientY - start.y,
        { w: window.innerWidth, h: window.innerHeight },
      ),
    );
  };

  const onResizeEnd = (event: ReactPointerEvent<HTMLDivElement>) => {
    resizeStart.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      // An open dropdown owns Escape: its window-level listener closes the
      // menu. Closing the whole dialog here would skip that layer.
      const doc = panelRef.current?.ownerDocument ?? document;
      if (doc.querySelector("[data-dropdown-panel]")) return;
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
    const body = bodyFromDraft(profile, bindableTools);
    if (!editing) {
      const bot = await createBot(trimmed, description.trim(), body);
      if (!bot) {
        setSaving(false);
        setError(i18n.t("bot.createFailed"));
        return;
      }
      // `bot/create` cannot carry a model; the session is created with its
      // seeded default. Apply the create-time draft as the first session knob.
      if (modelDraft && bot.session_id) {
        try {
          await useConnectionStore.getState().sendRpc("agent/set-model", {
            session_id: bot.session_id,
            model_id: modelDraft,
          });
        } catch (reason) {
          useToastStore.getState().showToast(
            `${i18n.t("bot.modelCreateFailed")} ${
              reason instanceof Error ? reason.message : ""
            }`.trim(),
            "error",
          );
        }
      }
      setSaving(false);
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
    if (!updated) {
      setSaving(false);
      setError(i18n.t("bot.updateFailed"));
      return;
    }
    // Same session knob as the composer. The draft is applied only after the
    // identity update succeeds, so a failed rename does not move the model.
    if (modelChanged && modelDraft && editing.session_id) {
      try {
        await useConnectionStore.getState().sendRpc("agent/set-model", {
          session_id: editing.session_id,
          model_id: modelDraft,
        });
      } catch (reason) {
        setSaving(false);
        setError(
          `${i18n.t("bot.modelUpdateFailed")} ${
            reason instanceof Error ? reason.message : ""
          }`.trim(),
        );
        return;
      }
    }
    setSaving(false);
    onClose();
  };

  const updateBinding = (toolId: string, patch: Partial<AgentToolBinding>) => {
    if (!profile) return;
    // Locked cards are spec-owned: a stray click must not write a row, whether
    // it came from the toggle or a preset button.
    if (bindableTools.find((tool) => tool.id === toolId)?.locked) return;
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
            className="relative flex max-h-[85vh] w-full max-w-[70rem] flex-col rounded-lg border border-(--_dk-line-visible) bg-(--_dk-overlay) text-(--_dk-text-secondary) shadow-(--_dk-elevation)"
            style={size ? { width: size.w, height: size.h } : undefined}
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
                    <div className="flex flex-wrap items-start gap-3">
                      <div className="min-w-[160px] flex-1">
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
                      <div className="min-w-[160px] flex-1">
                        <FieldLabel>{i18n.t("bot.model")}</FieldLabel>
                        <ModelSwitcher
                          variant="field"
                          modelId={modelId}
                          onChange={setModelDraft}
                          disabled={busy}
                        />
                      </div>
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
                  {loading ? (
                    <p className="text-xs text-(--_dk-text-disabled)">
                      {i18n.t("bot.loadingProfile")}
                    </p>
                  ) : profile ? (
                    <>
                      <AgentProfileFields
                        draft={profile}
                        saveBlocked={busy}
                        onChange={setProfile}
                      />
                      <AgentToolsGrid
                        draft={profile}
                        bindableTools={bindableTools}
                        mcpServers={mcpServers}
                        saveBlocked={busy}
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
                  disabled={saving || !name.trim() || !profile}
                >
                  {saving
                    ? i18n.t("bot.saving")
                    : editing
                      ? i18n.t("common.save")
                      : i18n.t("bot.create")}
                </button>
              </div>
            </form>
            {/* Drag the corner to resize; double-click restores content fit. */}
            <div
              data-resize-grip
              className="dialog-resize-grip"
              onPointerDown={onResizeStart}
              onPointerMove={onResizeMove}
              onPointerUp={onResizeEnd}
              onPointerCancel={onResizeEnd}
              onDoubleClick={() => setSize(null)}
              aria-hidden
            />
          </div>
        </div>,
        target,
      )}
    </>
  );
}
