import { isWorkspaceFileRef } from "./knowledge/markers";
import { useEditorStore } from "../stores/editorStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import {
  chipPath,
  dropClaimsForeign,
  dragOverClaimsForeign,
  osFilePath,
  readLoosePaths,
} from "./dropPayload";

export type ForeignZone = "tree" | "mention" | "editor" | "terminal" | "other";

export type ForeignIntent = "ignore" | "swallow" | "preview";

export function foreignZone(target: EventTarget | null): ForeignZone {
  const el = target instanceof Element ? target : null;
  if (!el) return "other";
  if (el.closest("[data-drop-zone='tree']")) return "tree";
  if (el.closest("[data-mention-drop]")) return "mention";
  const zone = el.closest("[data-drop-zone]")?.getAttribute("data-drop-zone");
  if (zone === "editor" || zone === "terminal") return zone;
  return "other";
}

/** Editor and welcome drops preview. Terminal drops are swallowed so a file name is not pasted. */
export function foreignDropIntent(
  target: EventTarget | null,
  dt: DataTransfer,
  phase: "over" | "drop",
): ForeignIntent {
  const claimed = phase === "over" ? dragOverClaimsForeign(dt) : dropClaimsForeign(dt);
  if (!claimed) return "ignore";
  const zone = foreignZone(target);
  if (zone === "terminal") return "swallow";
  if (zone === "editor") return "preview";
  return "ignore";
}

function isFolderPlaceholder(file: File): boolean {
  return file.size === 0 && !file.type;
}

/** Open dropped files as temporary previews, or a workspace path when there are no bytes. */
export async function previewForeignDrop(dt: DataTransfer): Promise<void> {
  const project = useSessionStore.getState().project;
  const files = Array.from(dt.files);
  if (files.length > 0) {
    const openable = files.filter((file) => !isFolderPlaceholder(file));
    if (openable.length === 0) {
      useToastStore
        .getState()
        .showToast("Folder drops from the OS are not supported", "info");
      return;
    }
    for (const file of openable) {
      const raw = osFilePath(file) ?? file.name;
      const chip = chipPath(raw || file.name, project);
      await useEditorStore.getState().openExternalPreview(file, chip);
    }
    return;
  }
  for (const raw of readLoosePaths(dt)) {
    const chip = chipPath(raw, project);
    if (isWorkspaceFileRef(chip)) {
      await useEditorStore.getState().openFile(chip);
    }
  }
}
