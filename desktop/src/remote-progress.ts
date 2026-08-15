export type RemoteProgressStage =
  | "authenticating"
  | "upload"
  | "verify"
  | "extract"
  | "ready"
  | "starting"
  | "attaching";

export type RemoteProgressEvent = {
  stage: RemoteProgressStage;
  /** 0..1 */
  ratio: number;
  message: string;
};

/** IPC channel for managed remote deploy progress (must match preload.ts literal). */
export const REMOTE_PROGRESS_CHANNEL = "litecode:remote-progress";
