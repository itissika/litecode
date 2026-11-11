declare class ToolError extends Error {
  readonly name: "ToolError";
  readonly code: string;
  readonly details?: unknown;
  readonly retryable?: true;
  readonly tool: string;
  readonly seq: number;
}

type ToolFailure = { // A resolved domain failure. ToolError is thrown for protocol and hard execution failures; this shape is the normal failure branch of ToolResult.
  ok: false;
  reason:
    | "not_found"
    | "no_match"
    | "invalid_args"
    | "empty"
    | "permission"
    | "io"
    | "conflict"
    | "stale"
    | "unavailable"
    | "still_alive";
  message: string;
};

/** A successful tool result. The host adds this envelope; payloads omit ok. */
type ToolSuccess<T> = T & { ok: true };

type ToolResult<T> = ToolSuccess<T> | ToolFailure;