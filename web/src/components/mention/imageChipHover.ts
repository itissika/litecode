import { readBytes } from "../../api/workspace";
import { fileKindFromPath } from "../../lib/fileKind";
import { isWorkspaceFileRef } from "../../lib/knowledge/markers";

export const IMAGE_CHIP_PREVIEW_WIDTH = 240;
export const IMAGE_CHIP_PREVIEW_HEIGHT = 160;
export const IMAGE_CHIP_PREVIEW_DELAY_MS = 180;

const previewUrls = new Map<string, string>();
const previewInflight = new Map<string, Promise<string>>();

/** Drop cached preview blobs. Tests use this between cases. */
export function clearImageChipPreviewCache(): void {
  for (const url of previewUrls.values()) URL.revokeObjectURL(url);
  previewUrls.clear();
  previewInflight.clear();
}

export function loadImageChipPreview(path: string): Promise<string> {
  const cached = previewUrls.get(path);
  if (cached) return Promise.resolve(cached);
  const pending = previewInflight.get(path);
  if (pending) return pending;
  const request = readBytes(path).then((blob) => {
    const url = URL.createObjectURL(blob);
    previewUrls.set(path, url);
    previewInflight.delete(path);
    return url;
  });
  previewInflight.set(path, request);
  request.catch(() => {
    previewInflight.delete(path);
  });
  return request;
}

/**
 * Path of a whole-file image chip, or null when this citation should stay a
 * text capsule. Symbol and line citations are locations, not the picture.
 * Outside-workspace paths have no bytes endpoint.
 */
export function imageChipPath(
  mode: "node" | "file",
  attrs: { id?: unknown; symbol?: unknown; lines?: unknown },
): string | null {
  if (mode !== "file") return null;
  const path = String(attrs.id ?? "").trim();
  if (!path || !isWorkspaceFileRef(path)) return null;
  if (String(attrs.symbol ?? "").trim()) return null;
  if (attrs.lines != null && String(attrs.lines).trim()) return null;
  if (fileKindFromPath(path) !== "image") return null;
  return path;
}
