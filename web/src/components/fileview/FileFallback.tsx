import { useState } from "react";

import { openWithDefaultApp } from "../../api/workspace";
import { useConnectionStore } from "../../stores/connectionStore";

/** Shown when the file panel cannot preview a path. Windows servers get a
 *  button that opens the file with the OS default application. */
export function FileFallback({
  path,
  message,
}: {
  path: string;
  message: string;
}) {
  const hostOs = useConnectionStore((s) => s.hostOs);
  const [pending, setPending] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
      <p className="max-w-md text-sm text-(--_dk-text)">{message}</p>
      {hostOs === "windows" ? (
        <button
          type="button"
          className="btn btn-sm"
          disabled={pending}
          onClick={() => {
            setPending(true);
            setOpenError(null);
            void openWithDefaultApp(path)
              .catch((error: unknown) => {
                setOpenError(
                  error instanceof Error ? error.message : String(error),
                );
              })
              .finally(() => setPending(false));
          }}
        >
          Open with default app
        </button>
      ) : null}
      {openError ? (
        <p className="max-w-md text-xs text-(--_dk-tag-danger-fg)">{openError}</p>
      ) : null}
    </div>
  );
}
