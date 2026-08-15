/**
 * Conflict card (FE-01 / DESIGN §3.5): shown when the agent overwrites a file
 * the user has open with unsaved edits. Names the conflicted file and its
 * source so the conflict is never silently skipped.
 */
export function ConflictCard({
  path,
  source,
  onDismiss,
}: {
  path: string;
  source: string;
  onDismiss: () => void;
}) {
  return (
    <div
      data-testid="conflict-card"
      role="alert"
      className="flex items-center justify-between gap-3 border-b border-(--_dk-amber-500)/40 bg-(--_dk-amber-500)/10 px-3 py-2 text-xs text-(--_dk-text-primary)"
    >
      <div className="min-w-0">
        <span className="font-medium">Unsaved changes conflicted with {source}</span>
        <span className="ml-1 break-all font-mono text-(--_dk-text-secondary)">
          {path}
        </span>
      </div>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss conflict"
        className="shrink-0 rounded px-1.5 text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-ix-fg-hover)"
      >
        Dismiss
      </button>
    </div>
  );
}
