declare class ToolError extends Error {
  readonly name: "ToolError";
  readonly code: string;
  readonly details?: unknown;
  readonly retryable?: true;
  readonly tool: string;
  readonly seq: number;
}

/** Any tool call resolves this when the tool did not get the job done. */
type ToolFailure = {
  ok: false;
  reason:
    | "not_found"
    | "no_match"
    | "invalid_args"
    | "empty"
    | "permission"
    | "io"
    | "conflict"
    | "stale";
  message: string;
};

type ToolResult<T> = T | ToolFailure;