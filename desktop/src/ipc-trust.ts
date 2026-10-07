import { isAllowedExternalUrl } from "./url-policy";

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
  if (surface === "workbench" && isPopoutPath(event.senderFrame.url)) {
    return { trusted: false, reason: "popout window cannot send IPC" };
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

const POPOUT_DOCK_ID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function isPopoutDockId(value: string): boolean {
  return POPOUT_DOCK_ID.test(value);
}

export type WindowOpenDecision = "popout" | "external" | "deny";

function parsedHttpUrl(raw: string): URL | null {
  try {
    const url = new URL(raw);
    if (
      (url.protocol !== "http:" && url.protocol !== "https:") ||
      url.username ||
      url.password
    ) {
      return null;
    }
    return url;
  } catch {
    return null;
  }
}

/** Workbench popout document. Pathname only — the query is checked separately. */
export function isWorkbenchPopoutDocument(
  raw: string,
  context: IpcTrustContext | null,
): boolean {
  if (!context || context.activeSurface !== "workbench" || !context.workbenchOrigin) {
    return false;
  }
  const url = parsedHttpUrl(raw);
  if (!url || url.origin !== context.workbenchOrigin) return false;
  return url.pathname === "/popout.html";
}

/** Dock id for a well-formed popout URL, or null when the window must not open. */
export function workbenchPopoutDockId(
  raw: string,
  context: IpcTrustContext | null,
): string | null {
  if (!isWorkbenchPopoutDocument(raw, context)) return null;
  const url = parsedHttpUrl(raw);
  if (!url || url.hash) return null;
  const keys = [...url.searchParams.keys()];
  if (keys.length !== 1 || keys[0] !== "dock") return null;
  const dock = url.searchParams.get("dock");
  if (!dock || !POPOUT_DOCK_ID.test(dock)) return null;
  return dock;
}

/**
 * `popout` opens a same-origin dock window. `external` is the existing
 * http(s)/mailto hand-off. Anything else, including a popout path without a
 * dock id, is denied and is not sent to the system browser.
 */
export function resolveWindowOpen(
  raw: string,
  context: IpcTrustContext | null,
): WindowOpenDecision {
  if (context && isWorkbenchPopoutDocument(raw, context)) {
    return workbenchPopoutDockId(raw, context) ? "popout" : "deny";
  }
  if (isAllowedExternalUrl(raw)) return "external";
  return "deny";
}

function isPopoutPath(raw: string): boolean {
  try {
    return new URL(raw).pathname === "/popout.html";
  } catch {
    return false;
  }
}
