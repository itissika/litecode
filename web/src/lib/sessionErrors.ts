import {
  RPC_ERR_AGENT_ALREADY_RUNNING,
  RPC_ERR_DESCENDANT_RUNNING,
  RPC_ERR_SUBAGENT_SESSION_READONLY,
} from "../api/types";
import i18n from "../i18n";

/** `error.data.code` string the server attaches to a rejected RPC, if any. */
export function rpcErrorCode(err: unknown): string | undefined {
  const data = (err as { data?: unknown } | null)?.data;
  if (!data || typeof data !== "object") return undefined;
  const code = (data as Record<string, unknown>).code;
  return typeof code === "string" ? code : undefined;
}

function runningSessionIds(err: unknown): string[] {
  const data = (err as { data?: unknown } | null)?.data;
  if (!data || typeof data !== "object") return [];
  const ids = (data as Record<string, unknown>).running_session_ids;
  return Array.isArray(ids)
    ? ids.filter((id): id is string => typeof id === "string")
    : [];
}

export function deleteFailedMessage(): string {
  return i18n.t("session.deleteFailed");
}

/** @deprecated Prefer deleteFailedMessage(); kept for call sites that need a string snapshot. */
export const DELETE_FAILED_MESSAGE = "Delete failed, please try again later";

/** User-facing text for a rejected `session/delete`. Unknown → generic. */
export function deleteRejectedMessage(err: unknown): string {
  switch (rpcErrorCode(err)) {
    case RPC_ERR_AGENT_ALREADY_RUNNING:
      return i18n.t("session.busyDelete");
    case RPC_ERR_DESCENDANT_RUNNING: {
      const n = runningSessionIds(err).length;
      return n > 0
        ? i18n.t("session.descendantsBusy", { count: n })
        : i18n.t("session.descendantBusy");
    }
    default:
      return i18n.t("session.deleteFailed");
  }
}

/** Readonly-rejection text, or undefined when `err` is another error. */
export function readonlyRejectedMessage(err: unknown): string | undefined {
  return rpcErrorCode(err) === RPC_ERR_SUBAGENT_SESSION_READONLY
    ? i18n.t("session.readonlyChild")
    : undefined;
}
