import { useEffect, useState } from "react";

import { fileNameFromPath } from "../../utils/language";
import { FileFallback } from "./FileFallback";
import { useWorkspaceBlob } from "./useWorkspaceBlob";

export function ImagePreview({
  path,
  diskRevision,
  sourceUrl,
}: {
  path: string;
  diskRevision: number;
  sourceUrl?: string;
}) {
  if (sourceUrl) return <ImageFrame path={path} url={sourceUrl} />;
  return <WorkspaceImage path={path} diskRevision={diskRevision} />;
}

function WorkspaceImage({
  path,
  diskRevision,
}: {
  path: string;
  diskRevision: number;
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
  return <ImageFrame path={path} url={url} />;
}

function ImageFrame({ path, url }: { path: string; url: string }) {
  const [broken, setBroken] = useState(false);
  useEffect(() => {
    setBroken(false);
  }, [url]);
  if (broken) {
    return <FileFallback path={path} message="Can't display this image here." />;
  }
  return (
    <div className="flex h-full items-center justify-center overflow-auto bg-(--_dk-editor) p-4">
      <img
        src={url}
        alt={fileNameFromPath(path)}
        className="max-h-full max-w-full object-contain"
        onError={() => setBroken(true)}
      />
    </div>
  );
}
