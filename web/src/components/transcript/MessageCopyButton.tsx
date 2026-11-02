import { Check, Copy } from "@phosphor-icons/react";
import { useEffect, useState } from "react";

/**
 * Copy one reply's source text — the text the model wrote, so citations stay
 * the shortcodes they were instead of the shortened capsule labels. A plain
 * selection of the rendered reply is the reading path; this is the faithful one.
 */
export function MessageCopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const label = copied ? "Copied" : "Copy message";

  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={() => {
        void navigator.clipboard
          .writeText(text)
          .then(() => setCopied(true))
          .catch(() => {});
      }}
      className="rounded bg-(--_dk-editor) p-1 text-(--_dk-text-muted) hover:text-(--_dk-text-secondary)"
    >
      {copied ? (
        <Check size={13} weight="bold" aria-hidden />
      ) : (
        <Copy size={13} aria-hidden />
      )}
    </button>
  );
}
