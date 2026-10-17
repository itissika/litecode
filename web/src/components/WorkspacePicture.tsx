import { useEffect, useState } from "react";

import { fileNameFromPath } from "../utils/language";
import { loadImageChipPreview } from "./mention/imageChipHover";

/**
 * In-flow picture for a workspace image. Bytes come from the chip preview
 * cache. A click handler is only for a citation that should open the file.
 */
export function WorkspacePicture({
  path,
  alt,
  onOpen,
}: {
  path: string;
  alt?: string;
  onOpen?: () => void;
}) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const label = alt?.trim() || fileNameFromPath(path);

  useEffect(() => {
    let cancelled = false;
    setUrl(null);
    setFailed(false);
    loadImageChipPreview(path).then(
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
  }, [path]);

  if (failed) {
    return label ? <span className="markdown-preview-figure">{label}</span> : null;
  }
  if (!url) return null;

  const img = <img src={url} alt={label} />;
  return (
    <span className="markdown-preview-figure">
      {onOpen ? (
        <button
          type="button"
          className="markdown-preview-figure-hit"
          aria-label={label}
          onClick={onOpen}
        >
          {img}
        </button>
      ) : (
        img
      )}
    </span>
  );
}
