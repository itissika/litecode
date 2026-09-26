import { useEffect, useState } from "react";

import type { FileKind } from "../../lib/fileKind";
import { fileNameFromPath } from "../../utils/language";
import { FileFallback } from "./FileFallback";
import { useWorkspaceBlob } from "./useWorkspaceBlob";

const CODEC_MESSAGE = "无法在面板中播放这个文件";

export function MediaPreview({
  path,
  diskRevision,
  kind,
}: {
  path: string;
  diskRevision: number;
  kind: Extract<FileKind, "audio" | "video">;
}) {
  const { url, error, loading } = useWorkspaceBlob(path, diskRevision);
  const [codecError, setCodecError] = useState(false);

  useEffect(() => {
    setCodecError(false);
  }, [url]);

  if (error) return <FileFallback path={path} message={error} />;
  if (codecError) return <FileFallback path={path} message={CODEC_MESSAGE} />;
  if (!url) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-muted)">
        {loading ? "Loading…" : "Waiting to reconnect…"}
      </div>
    );
  }

  const name = fileNameFromPath(path);
  return (
    <div className="flex h-full items-center justify-center overflow-auto bg-(--_dk-editor) p-4">
      {kind === "audio" ? (
        <audio
          src={url}
          controls
          aria-label={name}
          onError={() => setCodecError(true)}
        />
      ) : (
        <video
          src={url}
          controls
          aria-label={name}
          className="max-h-full max-w-full"
          onError={() => setCodecError(true)}
        />
      )}
    </div>
  );
}
