export type IpcSurface = "hub" | "workbench";
export type AllowedSurface = IpcSurface | "both";

export type IpcTrustContext = {
  activeSurface: IpcSurface;
  hubFileUrl: string;
  workbenchOrigin: string | null;
};

type FrameLike = { url: string };
type WebContentsLike = { mainFrame: FrameLike };
export type IpcEventLike = {
  sender: WebContentsLike;
  senderFrame: FrameLike | null;
};

export type SenderClassification =
  | { trusted: true; surface: IpcSurface }
  | { trusted: false; reason: string };

export function exactHttpOrigin(raw: string): string | null {
  try {
    const url = new URL(raw);
    if (
      (url.protocol !== "http:" && url.protocol !== "https:") ||
      url.username ||
      url.password
    ) {
      return null;
    }
    return url.origin;
  } catch {
    return null;
  }
}

export function classifySurfaceUrl(
  raw: string,
  context: IpcTrustContext,
): IpcSurface | null {
  if (raw === context.hubFileUrl) return "hub";
  if (!context.workbenchOrigin) return null;
  const origin = exactHttpOrigin(raw);
  return origin === context.workbenchOrigin ? "workbench" : null;
}

export function classifyIpcSender(
  event: IpcEventLike,
  trustedSender: WebContentsLike | null,
  context: IpcTrustContext,
): SenderClassification {
  if (!trustedSender || event.sender !== trustedSender) {
    return { trusted: false, reason: "sender does not own the trusted window" };
  }
  if (!event.senderFrame || event.senderFrame !== event.sender.mainFrame) {
    return { trusted: false, reason: "IPC from a subframe is not allowed" };
  }
  const surface = classifySurfaceUrl(event.senderFrame.url, context);
  if (!surface) {
    return { trusted: false, reason: "IPC sender URL is not trusted" };
  }
  if (surface !== context.activeSurface) {
    return { trusted: false, reason: "IPC sender is not the active surface" };
  }
  return { trusted: true, surface };
}

export function assertIpcSurface(
  event: IpcEventLike,
  trustedSender: WebContentsLike | null,
  context: IpcTrustContext,
  allowed: AllowedSurface,
): IpcSurface {
  const classification = classifyIpcSender(event, trustedSender, context);
  if (!classification.trusted) {
    throw new Error(`Rejected untrusted IPC: ${classification.reason}`);
  }
  if (allowed !== "both" && classification.surface !== allowed) {
    throw new Error(
      `Rejected IPC from ${classification.surface}; ${allowed} surface required`,
    );
  }
  return classification.surface;
}

export function isAllowedNavigation(
  surface: IpcSurface,
  raw: string,
  context: IpcTrustContext,
): boolean {
  if (surface === "hub") return raw === context.hubFileUrl;
  if (!context.workbenchOrigin) return false;
  return exactHttpOrigin(raw) === context.workbenchOrigin;
}
