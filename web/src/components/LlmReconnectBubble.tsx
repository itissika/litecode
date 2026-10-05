import { CircleNotch } from "@phosphor-icons/react";

import { formatLlmReconnect } from "../lib/llmReconnect";
import { useTurnStore } from "../stores/turnStore";
import { composerCardClass } from "./composerCard";

/**
 * Temporary capsule above the composer while an LLM call is reconnecting.
 * The failed state stays, with Retry, until the next turn starts.
 */
export function LlmReconnectBubble({
  sessionId,
  allowRetry = true,
}: {
  sessionId: string;
  /** Child sessions cannot `agent/retry`; the failure text still shows. */
  allowRetry?: boolean;
}) {
  const notice = useTurnStore(
    (s) => s.byId.get(sessionId)?.llmReconnect ?? null,
  );
  const retry = useTurnStore((s) => s.retryLlmReconnect);
  if (!notice || notice.phase === "cleared") return null;

  const failed = notice.phase === "failed";
  return (
    <div className="flex shrink-0 justify-center">
      <div
        data-testid="llm-reconnect-bubble"
        role="status"
        aria-live="polite"
        className={`${composerCardClass} inline-flex items-center gap-2 px-2.5 py-1 text-xs text-(--_dk-text-secondary)`}
      >
        {failed ? null : (
          <CircleNotch
            size={12}
            weight="bold"
            className="animate-spin"
            aria-hidden
          />
        )}
        <span>{formatLlmReconnect(notice)}</span>
        {failed && allowRetry ? (
          <button
            type="button"
            data-testid="llm-reconnect-retry"
            className="cursor-pointer font-medium text-(--_dk-text) underline-offset-2 hover:underline"
            onClick={() => retry(sessionId)}
          >
            Retry
          </button>
        ) : null}
      </div>
    </div>
  );
}
