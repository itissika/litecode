import { apiFetch } from "./auth";

export interface UploadedMedia {
  ref: string;
  mime: string;
  width: number;
  height: number;
}

/** Store a normalized image. The returned `ref` is `litecode-media:{sha256}.jpg`. */
export async function uploadMedia(blob: Blob): Promise<UploadedMedia> {
  const response = await apiFetch("/api/media", {
    method: "POST",
    headers: { "Content-Type": blob.type || "image/jpeg" },
    body: blob,
  });
  const body = (await response.json().catch(() => null)) as
    | (UploadedMedia & { ok?: boolean; error?: string })
    | null;
  if (!response.ok || !body?.ref) {
    throw new Error(body?.error || "Could not upload the image");
  }
  return {
    ref: body.ref,
    mime: body.mime,
    width: body.width,
    height: body.height,
  };
}
