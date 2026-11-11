/**
 * Raised when a call does not hold under the contract and the run has to stop.
 * It is uncatchable by design: a try/catch cannot turn it into a value.
 *
 * Fatal: a tool name that does not exist (a namespace typo), and a tool result
 * that violates its own declared output. Both mean the program's model of the
 * contract is wrong, not just one call.
 *
 * Not fatal, so the run continues: a result field the contract does not declare
 * is reported once, before the output, and reads as `undefined`; an argument
 * the input contract rejects resolves as `ToolFailure` with
 * `reason: "invalid_args"`, and the program can correct it on the next call.
 */
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
    | "ambiguous"
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