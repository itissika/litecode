import { useId } from "react";
import { useTranslation } from "react-i18next";

import type { AgentProfile } from "../../api/settings";
import { FieldLabel, TextArea } from "../ui/FormField";

/**
 * The bot editor's role box. Everything else on the bot (identity, base prompt,
 * core tools) is the locked `default` spec, so the user only writes the
 * `# Bot Role` segment that gets appended to that base prompt.
 */
export function AgentProfileFields({
  draft,
  saveBlocked,
  onChange,
}: {
  draft: AgentProfile;
  saveBlocked: boolean;
  onChange: (profile: AgentProfile) => void;
}) {
  const { t } = useTranslation();
  const fieldId = useId();
  const promptId = `${fieldId}-role-prompt`;
  return (
    <div className="settings-card space-y-2 p-3">
      <FieldLabel htmlFor={promptId}>{t("bot.rolePrompt")}</FieldLabel>
      <TextArea
        id={promptId}
        rows={4}
        value={draft.system_prompt}
        placeholder={t("bot.rolePromptPlaceholder")}
        onChange={(e) => onChange({ ...draft, system_prompt: e.target.value })}
        disabled={saveBlocked}
      />
    </div>
  );
}
