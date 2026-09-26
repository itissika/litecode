import { useEffect, useState } from "react";

import { apiFetch } from "../api/auth";

const MEDIA_NAME = /^[0-9a-f]{64}\.(?:jpg|png|webp|gif)$/;
const objectUrls = new Map<string, string>();
const inflight = new Map<string, Promise<string>>();

/** File name inside a `litecode-media:` ref, or null when it is not one. */
export function mediaNameFromRef(mediaRef: string): string | null {
  const name = mediaRef.startsWith("litecode-media:")
    ? mediaRef.slice("litecode-media:".length)
    : "";
  return MEDIA_NAME.test(name) ? name : null;
}

function loadMedia(mediaRef: string): Promise<string> {
  const cached = objectUrls.get(mediaRef);
  if (cached) return Promise.resolve(cached);
  const pending = inflight.get(mediaRef);
  if (pending) return pending;
  const name = mediaNameFromRef(mediaRef);
  if (!name) return Promise.reject(new Error("invalid image ref"));
  const request = apiFetch(`/api/media/${name}`, { cache: "no-store" }).then(async (response) => {
    if (!response.ok) throw new Error(String(response.status));
    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    objectUrls.set(mediaRef, url);
    inflight.delete(mediaRef);
    return url;
  });
  inflight.set(mediaRef, request);
  request.catch(() => {
    inflight.delete(mediaRef);
  });
  return request;
}

/**
 * Thumbnail of a stored image. The box is capped; the picture keeps its ratio.
 * A missing file, or a ref that is not ours, paints a cross.
 */
export function ImageThumb({
  mediaRef,
  masked = false,
  onRemove,
}: {
  mediaRef: string;
  masked?: boolean;
  onRemove?: () => void;
}) {
  const [url, setUrl] = useState<string | null>(
    () => objectUrls.get(mediaRef) ?? null,
  );
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const cached = objectUrls.get(mediaRef);
    if (cached) {
      setUrl(cached);
      setFailed(false);
      return;
    }
    let cancelled = false;
    setUrl(null);
    setFailed(false);
    loadMedia(mediaRef).then(
      (next) => {
        if (!cancelled) setUrl(next);
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [mediaRef]);

  return (
    <span className="relative inline-flex max-h-[120px] max-w-[160px] items-center justify-center overflow-hidden rounded-md border border-(--_dk-line) bg-(--_dk-bg-sunken)">
      {failed || !url ? (
        <span
          aria-label={failed ? "Image unavailable" : "Loading image"}
          className="flex h-[72px] w-[96px] items-center justify-center text-lg text-(--_dk-text-disabled)"
        >
          {failed ? "×" : ""}
        </span>
      ) : (
        <img
          src={url}
          alt=""
          onError={() => setFailed(true)}
          className="max-h-[120px] max-w-[160px] object-contain"
        />
      )}
      {masked ? (
        <span className="absolute inset-0 flex items-center justify-center bg-black/55 text-[11px] text-white">
          Unsupported
        </span>
      ) : null}
      {onRemove ? (
        <button
          type="button"
          aria-label="Remove image"
          onClick={onRemove}
          className="absolute top-0.5 right-0.5 flex h-4 w-4 items-center justify-center rounded-sm bg-black/55 text-[10px] leading-none text-white"
        >
          ×
        </button>
      ) : null}
    </span>
  );
}
