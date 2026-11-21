import { Check, Copy } from "@phosphor-icons/react";
import { useEffect, useMemo, useState } from "react";

import {
  getMarkdownHighlighter,
  isSupportedHighlightLang,
  normalizeLang,
  SHIKI_THEME_DARK,
  SHIKI_THEME_LIGHT,
} from "../lib/shiki";

const GENERIC_LANGS = new Set(["", "text", "txt", "plain", "plaintext"]);

/** Syntax-highlighted, copyable code block shared by chat and tool cards. */
export function CodeBlock({
  code,
  lang,
  streaming = false,
}: {
  code: string;
  lang: string;
  /** The fence is still being written. Highlighting is deferred to the seal:
   *  a streaming reply re-renders its tail every animation tick, and a full
   *  shiki pass per tick costs tens of milliseconds on a long block. */
  streaming?: boolean;
}) {
  const [html, setHtml] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const normalizedLang = useMemo(() => normalizeLang(lang), [lang]);
  const displayLang = lang || "text";
  const skipHighlight =
    GENERIC_LANGS.has(normalizedLang) ||
    !isSupportedHighlightLang(normalizedLang);

  useEffect(() => {
    if (skipHighlight || streaming) {
      // Functional write: the streaming tail re-renders on every tick and an
      // identical `setHtml(null)` would re-render this block for nothing.
      setHtml((prev) => (prev === null ? prev : null));
      return;
    }

    let cancelled = false;

    void (async () => {
      try {
        const highlighter = await getMarkdownHighlighter();
        const highlighted = highlighter.codeToHtml(code, {
          lang: normalizedLang,
          themes: { light: SHIKI_THEME_LIGHT, dark: SHIKI_THEME_DARK },
        });
        if (!cancelled) setHtml(highlighted);
      } catch {
        if (!cancelled) setHtml(null);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [code, normalizedLang, skipHighlight, streaming]);

  const copyCode = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      /* clipboard unavailable */
    }
  };

  return (
    <div className="agent-code-block mb-2 last:mb-0">
      <div className="agent-code-block-header">
        <span className="agent-code-block-lang">{displayLang}</span>
        <button
          type="button"
          onClick={() => void copyCode()}
          className="agent-code-block-copy"
          aria-label={copied ? "Copied" : "Copy code"}
        >
          {copied ? (
            <Check size={12} weight="bold" aria-hidden />
          ) : (
            <Copy size={12} aria-hidden />
          )}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      {html ? (
        <div
          className="agent-code-block-body shiki-host"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      ) : (
        <pre className="agent-code-block-fallback">
          <code>{code}</code>
        </pre>
      )}
    </div>
  );
}
