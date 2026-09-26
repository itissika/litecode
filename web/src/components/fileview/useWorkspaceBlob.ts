import { useEffect, useRef, useState } from "react";

import { readBytes } from "../../api/workspace";
import { useConnectionStore } from "../../stores/connectionStore";

/** Fetch preview bytes once per path revision. A disconnect does not drop a
 *  blob that already loaded; the next connect retries only a failed fetch. */
export function useWorkspaceBlob(path: string, diskRevision: number) {
  const connected = useConnectionStore((s) => s.state === "connected");
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const successKey = useRef<string | null>(null);
  const urlRef = useRef<string | null>(null);

  useEffect(() => {
    if (!connected) return;
    const key = `${path}\0${diskRevision}`;
    if (successKey.current === key && urlRef.current) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    void readBytes(path).then(
      (blob) => {
        if (cancelled) return;
        const next = URL.createObjectURL(blob);
        const prev = urlRef.current;
        urlRef.current = next;
        successKey.current = key;
        setUrl(next);
        setLoading(false);
        if (prev) URL.revokeObjectURL(prev);
      },
      (error: unknown) => {
        if (cancelled) return;
        successKey.current = null;
        setLoading(false);
        setError(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [connected, path, diskRevision]);

  useEffect(() => {
    return () => {
      if (urlRef.current) {
        URL.revokeObjectURL(urlRef.current);
        urlRef.current = null;
      }
      successKey.current = null;
    };
  }, [path]);

  return { url, error, loading };
}
