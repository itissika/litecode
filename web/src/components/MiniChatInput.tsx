import { type FormEvent, useEffect, useMemo, useRef } from "react";

import type { ContextMode, ThinkingTier } from "../api/types";
import { mentionKeysOf } from "../lib/knowledge/flowProjection";
import { useKnowledgeStore } from "../stores/knowledgeStore";
import { useSessionStore } from "../stores/sessionStore";
import { actionButtonGlass, composerCardClass } from "./composerCard";
import { AgentPicker, ContextModeToggle, ThinkSlider } from "./AgentChatInput";
import { ImageThumb } from "./ImageThumb";
import { ModelSwitcher } from "./ModelSwitcher";
import { MentionEditor, type MentionEditorHandle } from "./mention/MentionEditor";

export interface MiniChatInputSettings {
  primaryId: string;
  modelId: string;
  thinkingTier: ThinkingTier;
  contextMode: ContextMode;
}

export function MiniChatInput({
  sessionId,
  draft,
  images = [],
  settings,
  disabled = false,
  onDismiss,
  onChange,
  onSubmit,
}: {
  sessionId: string;
  draft: string;
  /** Images already on the message. This editor does not accept new ones. */
  images?: string[];
  settings: MiniChatInputSettings;
  disabled?: boolean;
  onDismiss: () => void;
  onChange: (draft: string, settings: MiniChatInputSettings) => void;
  onSubmit: (input: string, settings: MiniChatInputSettings) => void;
}) {
  const primaryAgents = useSessionStore((s) => s.primaryAgents);
  const editorRef = useRef<MentionEditorHandle>(null);
  const mentionKeys = useKnowledgeStore((s) => mentionKeysOf(s.nodes));
  const candidates = useMemo(
    () => mentionKeys.split("\n").filter((key) => key.length > 0),
    [mentionKeys],
  );

  useEffect(() => {
    editorRef.current?.focus();
  }, []);

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (disabled || !settings.modelId) return;
    if (!draft.trim() && images.length === 0) return;
    onSubmit(draft, settings);
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
              onChange={(primaryId) =>
                onChange(draft, { ...settings, primaryId })
              }
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
            onChange={(thinkingTier) =>
              onChange(draft, { ...settings, thinkingTier })
            }
          />
          <div className="mx-0.5 h-3.5 w-px shrink-0 bg-(--_dk-line)" />
          <ContextModeToggle
            mode={settings.contextMode}
            disabled={disabled}
            onChange={(contextMode) =>
              onChange(draft, { ...settings, contextMode })
            }
          />
        </div>
      </div>
      <div className="mx-3 border-t border-(--_dk-line)" />
      {images.length > 0 ? (
        <div className="flex flex-wrap gap-1.5 px-3 pt-2">
          {images.map((ref, index) => (
            <ImageThumb key={`${ref}:${index}`} mediaRef={ref} />
          ))}
        </div>
      ) : null}
      {/* Same split as AgentChatInput: the draft scrolls one level in, so the
          outer box (which holds the absolutely positioned send button) never
          scrolls and the button cannot drift with the text. */}
      <div className="relative flex max-h-64 flex-col overflow-hidden">
        <div className="min-h-0 flex-auto overflow-y-auto">
          <MentionEditor
            handle={editorRef}
            label="Edit and resend"
            sourceId={sessionId}
            value={draft}
            candidates={candidates}
            placeholder="Revert and resend…"
            className="mention-composer-input w-full px-3 py-2 pr-12 text-sm text-(--_dk-text-primary)"
            symbolLines
            submitOnEnter={!disabled}
            onChange={(next) => onChange(next, settings)}
            onSubmit={() => submit()}
            onEscape={onDismiss}
            onMentionDrop
          />
        </div>
        <button
          type="submit"
          disabled={
            disabled ||
            !settings.modelId ||
            (!draft.trim() && images.length === 0)
          }
          className={`${actionButtonGlass} absolute right-2 bottom-2 flex h-[30px] w-[30px] items-center justify-center rounded-md border border-(--_dk-border-strong) text-(--_dk-text-primary) transition-transform duration-100 hover:brightness-110 active:scale-90 disabled:cursor-not-allowed disabled:opacity-40`}
          title="Revert and resend"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M5 12h14M12 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </form>
  );
}
