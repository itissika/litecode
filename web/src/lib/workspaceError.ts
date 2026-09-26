/** HTTP failure from a workspace route, with enough status to decide a retry. */
export class WorkspaceRequestError extends Error {
  readonly status: number;
  readonly retryable: boolean;

  constructor(message: string, status: number) {
    super(message);
    this.name = "WorkspaceRequestError";
    this.status = status;
    this.retryable = status === 0 || status >= 500;
  }
}

/** 413 / 415: the bytes are not a text document the editor can show. */
export function undisplayableStatus(status: number): boolean {
  return status === 413 || status === 415;
}
