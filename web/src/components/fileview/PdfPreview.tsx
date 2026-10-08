import { useEffect, useRef, useState } from "react";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

import { readBytes } from "../../api/workspace";
import { useConnectionStore } from "../../stores/connectionStore";
import { FileFallback } from "./FileFallback";

/** Lazy entry: pdfjs stays out of the main bundle until a PDF tab opens. */
export function PdfPreview({
  path,
  diskRevision,
  sourceUrl,
}: {
  path: string;
  diskRevision: number;
  sourceUrl?: string;
}) {
  const connected = useConnectionStore((s) => s.state === "connected");
  const ready = Boolean(sourceUrl) || connected;
  const hostRef = useRef<HTMLDivElement>(null);
  const successKey = useRef<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [readyPages, setReadyPages] = useState(false);

  useEffect(() => {
    if (!ready) return;
    const key = `${sourceUrl ?? path}\0${diskRevision}`;
    if (successKey.current === key) return;
    const host = hostRef.current;
    if (!host) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    void (async () => {
      try {
        const blob = sourceUrl
          ? await fetch(sourceUrl).then((response) => response.blob())
          : await readBytes(path);
        const data = new Uint8Array(await blob.arrayBuffer());
        const pdfjs = await import("pdfjs-dist");
        pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;
        const task = pdfjs.getDocument({ data });
        const doc = await task.promise;
        if (cancelled) {
          await task.destroy();
          return;
        }
        host.replaceChildren();
        for (let pageNumber = 1; pageNumber <= doc.numPages; pageNumber += 1) {
          if (cancelled) break;
          const page = await doc.getPage(pageNumber);
          const viewport = page.getViewport({ scale: 1.25 });
          const canvas = document.createElement("canvas");
          canvas.width = viewport.width;
          canvas.height = viewport.height;
          canvas.className = "mx-auto mb-3 max-w-full bg-white";
          host.appendChild(canvas);
          await page.render({ canvas, viewport }).promise;
          page.cleanup();
        }
        await task.destroy();
        if (cancelled) return;
        successKey.current = key;
        setReadyPages(true);
        setLoading(false);
      } catch (err) {
        if (cancelled) return;
        successKey.current = null;
        setLoading(false);
        setError(err instanceof Error ? err.message : String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [ready, path, diskRevision, sourceUrl]);

  useEffect(() => {
    successKey.current = null;
    setReadyPages(false);
  }, [path, diskRevision, sourceUrl]);

  if (error) return <FileFallback path={path} message={error} />;

  return (
    <div className="relative h-full overflow-auto bg-(--_dk-editor) p-4">
      {loading ? (
        <div className="absolute inset-0 flex items-center justify-center text-sm text-(--_dk-text-muted)">
          Loading…
        </div>
      ) : null}
      {!ready && !loading && !readyPages ? (
        <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-muted)">
          Waiting to reconnect…
        </div>
      ) : null}
      <div ref={hostRef} />
    </div>
  );
}
