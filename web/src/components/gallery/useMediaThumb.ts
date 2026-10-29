import { useEffect, useState } from "react";

import {
  MEDIA_THUMB_VERSION,
  fetchMediaThumb,
  type MediaEntry,
} from "../../api/workspace";

/**
 * Object-URL cache for Gallery thumbnails.
 *
 * The server caches decoded thumbs on disk and names them by path + mtime + size,
 * so a reload after an edit asks for a different URL. Here we keep a small LRU of
 * finished object URLs: scrolling back and forth does not re-decode or re-fetch.
 * Mounted tiles pin their URL, so eviction never blanks a thumbnail on screen.
 * Videos and audio have no thumbnail and never enter this cache.
 */

const CACHE_LIMIT = 240;
const THUMB_EDGE = 320;

const urls = new Map<string, string>();
const inflight = new Map<string, Promise<string>>();
/** Keys currently rendered by a mounted tile; never revoked while pinned. */
const pinned = new Map<string, number>();

export interface ThumbLookup {
  url: string | null;
  failed: boolean;
}

const EMPTY: ThumbLookup = { url: null, failed: false };

export function mediaThumbKey(entry: MediaEntry): string {
  // The encoder version belongs in the key as much as the file revision: a
  // blob URL built by an older encoder must not survive a version bump.
  return `${entry.path}|${entry.size}|${entry.mtime}|${MEDIA_THUMB_VERSION}`;
}

function pin(key: string): void {
  pinned.set(key, (pinned.get(key) ?? 0) + 1);
}

function unpin(key: string): void {
  const count = (pinned.get(key) ?? 0) - 1;
  if (count <= 0) pinned.delete(key);
  else pinned.set(key, count);
}

function remember(key: string, url: string): void {
  // Refresh recency: delete first so the Map keeps LRU order on re-set.
  urls.delete(key);
  urls.set(key, url);
  if (urls.size <= CACHE_LIMIT) return;
  for (const candidate of urls.keys()) {
    if (urls.size <= CACHE_LIMIT) break;
    if (candidate === key || pinned.has(candidate)) continue;
    const stale = urls.get(candidate);
    urls.delete(candidate);
    if (stale) URL.revokeObjectURL(stale);
  }
}

function load(
  key: string,
  path: string,
  mtime: number,
  size: number,
): Promise<string> {
  const cached = urls.get(key);
  if (cached) {
    remember(key, cached);
    return Promise.resolve(cached);
  }
  const pending = inflight.get(key);
  if (pending) return pending;

  const request = fetchMediaThumb(path, mtime, size, THUMB_EDGE)
    .then((blob) => {
      const url = URL.createObjectURL(blob);
      remember(key, url);
      return url;
    })
    .finally(() => {
      inflight.delete(key);
    });
  inflight.set(key, request);
  return request;
}

export function useMediaThumb(entry: MediaEntry): ThumbLookup {
  const key = entry.kind === "image" ? mediaThumbKey(entry) : "";
  const [state, setState] = useState<{ key: string; lookup: ThumbLookup }>({
    key: "",
    lookup: EMPTY,
  });

  useEffect(() => {
    if (!key) return;
    pin(key);
    let alive = true;
    load(key, entry.path, entry.mtime, entry.size)
      .then((url) => {
        if (alive) setState({ key, lookup: { url, failed: false } });
      })
      .catch(() => {
        if (alive) setState({ key, lookup: { url: null, failed: true } });
      });
    return () => {
      alive = false;
      unpin(key);
    };
  }, [key, entry.path, entry.mtime, entry.size]);

  if (!key || state.key !== key) return EMPTY;
  return state.lookup;
}
