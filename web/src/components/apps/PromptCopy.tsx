import { useState } from "react";

import { Check, Copy } from "@phosphor-icons/react";

import i18n from "../../i18n";
import { useToastStore } from "../../stores/toastStore";

/** A message the human hands to the agent, copied with one small icon. */
export function PromptCopy({
  title,
  hint,
  text,
}: {
  title?: string;
  hint?: string;
  text: string;
}) {
  const showToast = useToastStore((state) => state.showToast);
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      showToast(i18n.t("apps.copied"), "info");
    } catch (error) {
      showToast(error instanceof Error ? error.message : String(error), "error");
    }
  };

  const label = copied ? i18n.t("apps.copied") : i18n.t("apps.copyPrompt");

  return (
    <div className="apps-prompt">
      {title ? <div className="apps-section-title">{title}</div> : null}
      {hint ? <p className="apps-hint">{hint}</p> : null}
      <div className="apps-prompt-row">
        <code className="apps-prompt-text">{text}</code>
        <button
          type="button"
          className="apps-icon-button apps-copy-button"
          aria-label={label}
          title={label}
          onClick={() => void copy()}
        >
          {copied ? <Check size={13} /> : <Copy size={13} />}
        </button>
      </div>
    </div>
  );
}
