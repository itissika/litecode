export const BOT_FOLDER_PREFIX = "bot-folder-";

export type FolderScope = "bot" | "session";

/** Bot folders are session-folder rows generated with a reserved id prefix. */
export function isBotFolderId(id: string | null | undefined): boolean {
  return typeof id === "string" && id.startsWith(BOT_FOLDER_PREFIX);
}

export function folderScope(id: string | null | undefined): FolderScope {
  return isBotFolderId(id) ? "bot" : "session";
}
