export const SESSION_DRAG_MIME = "application/x-litecode-session-id";
export const BOT_DRAG_MIME = "application/x-litecode-bot-id";
export const SESSION_FOLDER_DRAG_MIME =
  "application/x-litecode-session-folder-id";

export interface SessionDragPayload {
  kind: "session";
  id: string;
}

export interface BotDragPayload {
  kind: "bot";
  id: string;
}

export interface SessionFolderDragPayload {
  kind: "session-folder";
  id: string;
}

function parsePayload<T extends { kind: string; id: string }>(
  raw: string,
  kind: T["kind"],
): T | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (
      parsed &&
      typeof parsed === "object" &&
      (parsed as { kind?: unknown }).kind === kind &&
      typeof (parsed as { id?: unknown }).id === "string"
    ) {
      return parsed as T;
    }
  } catch {
    // Foreign or malformed payload: not one of ours.
  }
  return null;
}

export function parseSessionDrag(raw: string): SessionDragPayload | null {
  return parsePayload<SessionDragPayload>(raw, "session");
}

export function parseBotDrag(raw: string): BotDragPayload | null {
  return parsePayload<BotDragPayload>(raw, "bot");
}

export function parseSessionFolderDrag(
  raw: string,
): SessionFolderDragPayload | null {
  return parsePayload<SessionFolderDragPayload>(raw, "session-folder");
}
