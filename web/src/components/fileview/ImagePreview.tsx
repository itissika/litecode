import { useEffect, useState } from "react";

import { fileNameFromPath } from "../../utils/language";
import { FileFallback } from "./FileFallback";
import { useWorkspaceBlob } from "./useWorkspaceBlob";

export function ImagePreview({
  path,
  diskRevision,
}: {
  path: string;
  diskRevision: number;
}) {
  const { url, error, loading } = useWorkspaceBlob(path, diskRevision);
  const [broken, setBroken] = useState(false);
  useEffect(() => {
    setBroken(false);
  }, [url]);
  if (error) return <FileFallback path={path} message={error} />;
  if (broken) {
    return <FileFallback path={path} message="无法在这里显示这张图片" />;
  }
  if (!url) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-muted)">
        {loading ? "Loading…" : "Waiting to reconnect…"}
      </div>
    );
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
