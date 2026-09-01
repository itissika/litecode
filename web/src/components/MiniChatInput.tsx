import { type FormEvent, type KeyboardEvent, useEffect, useLayoutEffect, useRef } from "react";

import type { ContextMode, ThinkingTier } from "../api/types";
import { useSessionStore } from "../stores/sessionStore";
import { actionButtonGlass, composerCardClass } from "./composerCard";
import { AgentPicker, ContextModeToggle, ThinkSlider } from "./AgentChatInput";
import { ModelSwitcher } from "./ModelSwitcher";

export interface MiniChatInputSettings {
  primaryId: string;
  modelId: string;
  thinkingTier: ThinkingTier;
  contextMode: ContextMode;
}

export function MiniChatInput({
  sessionId,
  draft,
  settings,
  disabled = false,
  onDismiss,
  onChange,
  onSubmit,
}: {
  sessionId: string;
  draft: string;
  settings: MiniChatInputSettings;
  disabled?: boolean;
  onDismiss: () => void;
  onChange: (draft: string, settings: MiniChatInputSettings) => void;
  onSubmit: (input: string, settings: MiniChatInputSettings) => void;
}) {
  const primaryAgents = useSessionStore((s) => s.primaryAgents);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const resizeTextarea = () => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${Math.min(textarea.scrollHeight, 256)}px`;
  };

  useEffect(() => {
    textareaRef.current?.focus();
  }, []);
  useLayoutEffect(() => {
    resizeTextarea();
  }, [draft]);

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (disabled || !draft.trim() || !settings.modelId) return;
    onSubmit(draft, settings);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onDismiss();
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  };

  return (
    <form
      data-testid="mini-chat-input"
      data-mini-chat-input
      onSubmit={submit}
      className={`${composerCardClass} my-1`}
    >
      <div className="flex min-w-0 items-center gap-1 overflow-hidden px-1.5 py-1">
        <div className="flex min-w-0 flex-1 items-center gap-1">
          {primaryAgents.length > 0 ? (
            <AgentPicker
              agents={primaryAgents}
              activeId={settings.primaryId}
              pendingId={null}
              disabled={disabled}
              onChange={(primaryId) => onChange(draft, { ...settings, primaryId })}
            />
          ) : null}
          <ModelSwitcher
            sessionId={sessionId}
            disabled={disabled}
            modelId={settings.modelId || null}
            onChange={(modelId) => onChange(draft, { ...settings, modelId })}
          />
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <ThinkSlider
            sessionId={`mini-${sessionId}`}
            value={settings.thinkingTier}
            disabled={disabled}
            onChange={(thinkingTier) => onChange(draft, { ...settings, thinkingTier })}
          />
          <div className="mx-0.5 h-3.5 w-px shrink-0 bg-(--_dk-line)" />
          <ContextModeToggle
            mode={settings.contextMode}
            disabled={disabled}
            onChange={(contextMode) => onChange(draft, { ...settings, contextMode })}
          />
        </div>
      </div>
      <div className="mx-3 border-t border-(--_dk-line)" />
      <div className="relative">
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={(event) => {
            onChange(event.target.value, settings);
            resizeTextarea();
          }}
          onKeyDown={onKeyDown}
          rows={2}
          placeholder="Edit and resend..."
          className="w-full max-h-64 resize-none overflow-y-auto border-0 bg-transparent px-3 py-2 pr-12 text-sm text-(--_dk-text-primary) outline-none placeholder:text-(--_dk-text-disabled) focus-visible:shadow-none"
        />
        <button
          type="submit"
          disabled={disabled || !draft.trim() || !settings.modelId}
          className={`${actionButtonGlass} absolute right-2 bottom-2 flex h-[30px] w-[30px] items-center justify-center rounded-md border border-(--_dk-border-strong) text-(--_dk-text-primary) transition-transform duration-100 hover:brightness-110 active:scale-90 disabled:cursor-not-allowed disabled:opacity-40`}
          title="Revert and resend"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M5 12h14M12 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </form>
  );
}
