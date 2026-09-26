/** Longest side of a composer image after normalize, in pixels. */
export const IMAGE_LONG_SIDE = 1568;

/** JPEG quality for the stored asset. */
export const JPEG_QUALITY = 0.85;

/** How many images one composer message may hold. Matches the server cap. */
export const MAX_COMPOSER_IMAGES = 16;

export function fittedSize(
  width: number,
  height: number,
  longSide = IMAGE_LONG_SIDE,
): { width: number; height: number } {
  const longest = Math.max(width, height, 1);
  if (longest <= longSide) {
    return {
      width: Math.max(1, Math.round(width)),
      height: Math.max(1, Math.round(height)),
    };
  }
  const scale = longSide / longest;
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

/** Decode, shrink the long side, and re-encode as JPEG on a white background. */
export async function normalizeImage(file: Blob): Promise<Blob> {
  const bitmap = await createImageBitmap(file);
  try {
    const { width, height } = fittedSize(bitmap.width, bitmap.height);
    const canvas =
      typeof OffscreenCanvas !== "undefined"
        ? new OffscreenCanvas(width, height)
        : Object.assign(document.createElement("canvas"), { width, height });
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not prepare the image");
    context.fillStyle = "#ffffff";
    context.fillRect(0, 0, width, height);
    context.drawImage(bitmap, 0, 0, width, height);
    if (canvas instanceof OffscreenCanvas) {
      return await canvas.convertToBlob({
        type: "image/jpeg",
        quality: JPEG_QUALITY,
      });
    }
    const blob = await new Promise<Blob | null>((resolve) => {
      canvas.toBlob(resolve, "image/jpeg", JPEG_QUALITY);
    });
    if (!blob) throw new Error("Could not prepare the image");
    return blob;
  } finally {
    bitmap.close();
  }
}

/** Image files on a paste or drop. Text on the same clipboard is left alone when this is empty. */
export function clipboardImageFiles(data: DataTransfer | null): File[] {
  if (!data) return [];
  const files: File[] = [];
  for (const item of data.items) {
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const file = item.getAsFile();
      if (file) files.push(file);
    }
  }
  if (files.length === 0) {
    for (const file of data.files) {
      if (file.type.startsWith("image/")) files.push(file);
    }
  }
  return files;
}
