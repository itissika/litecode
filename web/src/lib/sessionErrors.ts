import {
  RPC_ERR_AGENT_ALREADY_RUNNING,
  RPC_ERR_DESCENDANT_RUNNING,
  RPC_ERR_SUBAGENT_SESSION_READONLY,
} from "../api/types";

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

export const DELETE_FAILED_MESSAGE = "删除失败，请稍后重试";
export const READONLY_MESSAGE = "子会话是只读的，消息只能由父会话发送";

/** User-facing text for a rejected `session/delete`. Unknown → generic. */
export function deleteRejectedMessage(err: unknown): string {
  switch (rpcErrorCode(err)) {
    case RPC_ERR_AGENT_ALREADY_RUNNING:
      return "会话正在运行，请先停止再删除";
    case RPC_ERR_DESCENDANT_RUNNING: {
      const n = runningSessionIds(err).length;
      return n > 0
        ? `有 ${n} 个子会话正在运行，请先停止再删除`
        : "有子会话正在运行，请先停止再删除";
    }
    default:
      return DELETE_FAILED_MESSAGE;
  }
}

/** Readonly-rejection text, or undefined when `err` is another error. */
export function readonlyRejectedMessage(err: unknown): string | undefined {
  return rpcErrorCode(err) === RPC_ERR_SUBAGENT_SESSION_READONLY
    ? READONLY_MESSAGE
    : undefined;
}
