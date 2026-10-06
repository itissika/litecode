import { useEffect, useState } from "react";

import { readSqlitePreview, type SqlitePreview } from "../../api/workspace";
import { useConnectionStore } from "../../stores/connectionStore";
import { FileFallback } from "./FileFallback";

function cellText(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

export function SqlitePreview({
  path,
  diskRevision,
}: {
  path: string;
  diskRevision: number;
}) {
  const connected = useConnectionStore((s) => s.state === "connected");
  const [table, setTable] = useState("");
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<SqlitePreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setTable("");
    setOffset(0);
  }, [path, diskRevision]);

  useEffect(() => {
    if (!connected) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    void readSqlitePreview(path, table, offset).then(
      (next) => {
        if (cancelled) return;
        setPage(next);
        setLoading(false);
      },
      (err: unknown) => {
        if (cancelled) return;
        setLoading(false);
        setError(err instanceof Error ? err.message : String(err));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [connected, path, diskRevision, table, offset]);

  if (error && !page) return <FileFallback path={path} message={error} />;

  const columns = page?.columns ?? [];
  const rows = page?.rows ?? [];
  const tables = page?.tables ?? [];
  const activeTable = table || page?.table || "";

  return (
    <div className="flex h-full min-h-0 flex-col bg-(--_dk-editor)">
      <div className="flex shrink-0 items-center gap-2 border-b border-(--_dk-line-visible) px-3 py-1.5">
        <label className="flex items-center gap-2 text-xs text-(--_dk-text-muted)">
          Table
          <select
            className="rounded border border-(--_dk-line) bg-(--_dk-bg) px-2 py-1 text-xs text-(--_dk-text)"
            value={activeTable}
            onChange={(event) => {
              setTable(event.target.value);
              setOffset(0);
            }}
          >
            {tables.length === 0 ? <option value=""> </option> : null}
            {tables.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <span className="ml-auto text-xs text-(--_dk-text-muted)">
          {loading ? "Loading…" : null}
        </span>
        <button
          type="button"
          className="btn btn-xs"
          disabled={offset === 0 || loading}
          onClick={() => setOffset((n) => Math.max(0, n - (page?.limit ?? 100)))}
        >
          Prev
        </button>
        <button
          type="button"
          className="btn btn-xs"
          disabled={!page?.truncated || loading}
          onClick={() => setOffset((n) => n + (page?.limit ?? 100))}
        >
          Next
        </button>
      </div>
      {error ? (
        <div className="border-b border-(--_dk-tag-danger-border) bg-(--_dk-tag-danger-bg) px-3 py-1 text-xs text-(--_dk-tag-danger-fg)">
          {error}
        </div>
      ) : null}
      {page && tables.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-sm text-(--_dk-text-muted)">
          This database has no tables.
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <table className="w-full border-collapse text-left text-xs">
            <thead className="sticky top-0 bg-(--_dk-bg)">
              <tr>
                {columns.map((column) => (
                  <th
                    key={column}
                    className="border-b border-(--_dk-line) px-2 py-1 font-medium text-(--_dk-text-muted)"
                  >
                    {column}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rowIndex) => (
                <tr key={`${offset}-${rowIndex}`}>
                  {row.map((cell, cellIndex) => (
                    <td
                      key={`${columns[cellIndex] ?? cellIndex}`}
                      className="max-w-80 truncate border-b border-(--_dk-line-visible) px-2 py-1 text-(--_dk-text)"
                      title={cellText(cell)}
                    >
                      {cellText(cell)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
