import { useEffect, useState } from "react";

import type { FileKind } from "../../lib/fileKind";
import { fileNameFromPath } from "../../utils/language";
import { FileFallback } from "./FileFallback";
import { useWorkspaceBlob } from "./useWorkspaceBlob";

const CODEC_MESSAGE = "Can't play this file in the panel.";

export function MediaPreview({
  path,
  diskRevision,
  kind,
  sourceUrl,
}: {
  path: string;
  diskRevision: number;
  kind: Extract<FileKind, "audio" | "video">;
  sourceUrl?: string;
}) {
  if (sourceUrl) return <MediaFrame path={path} url={sourceUrl} kind={kind} />;
  return <WorkspaceMedia path={path} diskRevision={diskRevision} kind={kind} />;
}

function WorkspaceMedia({
  path,
  diskRevision,
  kind,
}: {
  path: string;
  diskRevision: number;
  kind: Extract<FileKind, "audio" | "video">;
}) {
  const { url, error, loading } = useWorkspaceBlob(path, diskRevision);
  if (error) return <FileFallback path={path} message={error} />;
  if (!url) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-muted)">
        {loading ? "Loading…" : "Waiting to reconnect…"}
      </div>
    );
  }
  return <MediaFrame path={path} url={url} kind={kind} />;
}

function MediaFrame({
  path,
  url,
  kind,
}: {
  path: string;
  url: string;
  kind: Extract<FileKind, "audio" | "video">;
}) {
  const [codecError, setCodecError] = useState(false);
  useEffect(() => {
    setCodecError(false);
  }, [url]);
  if (codecError) return <FileFallback path={path} message={CODEC_MESSAGE} />;
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
