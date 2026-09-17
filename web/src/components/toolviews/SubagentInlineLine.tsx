import { openSubagentPanel } from "../../lib/sessionPanelNav";

export function SubagentInlineLine({
  label,
  secondary,
  statusText,
  failed = false,
  childId,
  testId,
}: {
  label: string;
  secondary?: string;
  statusText: string;
  failed?: boolean;
  childId?: string;
  testId: string;
}) {
  const inner = (
    <>
      <span className="shrink-0 font-mono text-(--_dk-text-primary)">{label}</span>
      {secondary ? (
        <span className="min-w-0 truncate text-(--_dk-text-secondary)">{secondary}</span>
      ) : null}
      <span
        className={`min-w-0 truncate ${
          failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-muted)"
        }`}
      >
        {statusText}
      </span>
    </>
  );
  if (!childId) {
    return (
      <div className="flex min-w-0 items-center gap-1.5" data-testid={testId}>
        {inner}
      </div>
    );
  }
  return (
    <button
      type="button"
      onClick={() => openSubagentPanel(childId)}
      className="flex min-w-0 items-center gap-1.5"
      data-testid={testId}
    >
      {inner}
    </button>
  );
}
