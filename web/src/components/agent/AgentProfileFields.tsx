import { useId } from "react";
import { useTranslation } from "react-i18next";

import type { AgentProfile } from "../../api/settings";
import { FieldLabel, TextArea, TextInput } from "../ui/FormField";

/** Agent prompt form. Shared by Settings → Agents and the bot editor. */
export function AgentProfileFields({
  draft,
  saveBlocked,
  onChange,
  showDescription = true,
}: {
  draft: AgentProfile;
  saveBlocked: boolean;
  onChange: (profile: AgentProfile) => void;
  /** Bot identity owns its own description field, so the bot editor hides this one. */
  showDescription?: boolean;
}) {
  const { t } = useTranslation();
  const fieldId = useId();
  const descriptionId = `${fieldId}-description`;
  const promptId = `${fieldId}-system-prompt`;
  return (
    <div className="settings-card space-y-2 p-3">
      {showDescription ? (
        <div>
          <FieldLabel htmlFor={descriptionId}>{t("common.description")}</FieldLabel>
          {draft.role === "subagent" ? (
            <TextArea
              id={descriptionId}
              rows={4}
              value={draft.description}
              onChange={(e) =>
                onChange({ ...draft, description: e.target.value })
              }
              disabled={saveBlocked}
            />
          ) : (
            <TextInput
              id={descriptionId}
              value={draft.description}
              onChange={(e) =>
                onChange({ ...draft, description: e.target.value })
              }
              disabled={saveBlocked}
            />
          )}
        </div>
      ) : null}
      <div>
        <FieldLabel htmlFor={promptId}>
          {t("settings.agents.systemPrompt")}
        </FieldLabel>
        <TextArea
          id={promptId}
          rows={4}
          value={draft.system_prompt}
          onChange={(e) =>
            onChange({ ...draft, system_prompt: e.target.value })
          }
          disabled={saveBlocked}
        />
        <p className="mt-1 text-[11px] text-(--_dk-text-muted)">
          {t("settings.agents.systemPromptHint")}
        </p>
      </div>
    </div>
  );
}
