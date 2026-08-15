import { WarningCircle } from "@phosphor-icons/react";

interface PermissionCardProps {
  tool: string;
  ruleId: string;
  summary: string;
  requestId?: string;
  onGrant: (approved: boolean, always: boolean) => void;
}

export function PermissionCard({
  tool,
  ruleId,
  summary,
  requestId,
  onGrant,
}: PermissionCardProps) {
  const requestSuffix = requestId?.slice(-8);

  return (
    <div
      className="mx-3 my-2 flex gap-3 rounded-md border border-(--_dk-line-visible) border-l-2 border-l-(--_dk-amber-500) bg-(--_dk-surface-raised) p-3 shadow-(--_dk-elevation)"
      role="group"
      aria-labelledby="perm-title"
      aria-describedby={requestSuffix ? "perm-request-id" : undefined}
      data-testid="permission-card"
    >
      <WarningCircle
        size={18}
        weight="fill"
        className="mt-0.5 shrink-0 text-(--_dk-amber-500)"
        aria-hidden
      />
      <div className="min-w-0 flex-1">
        <h2
          id="perm-title"
          className="text-dk-md font-semibold text-(--_dk-text-primary)"
        >
          Permission required
        </h2>
        <p className="mt-1 text-dk-sm text-(--_dk-text-secondary)">
          The agent wants to run{" "}
          <span className="font-mono text-(--_dk-text-primary)">{tool}</span>
        </p>
        <p className="mt-1 text-dk-xs text-(--_dk-text-muted)">{summary}</p>
        <p className="mt-0.5 font-mono text-dk-2xs text-(--_dk-text-disabled)">
          Rule: {ruleId}
        </p>
        {requestSuffix && (
          <p
            id="perm-request-id"
            className="mt-1 font-mono text-dk-2xs text-(--_dk-text-disabled)"
          >
            Request: …{requestSuffix}
          </p>
        )}
        <div className="mt-3 flex flex-wrap gap-2">
          <button
            type="button"
            onClick={() => onGrant(true, false)}
            className="btn-primary"
          >
            Allow once
          </button>
          <button
            type="button"
            onClick={() => onGrant(true, true)}
            className="btn-ghost"
          >
            Always allow
          </button>
          <button
            type="button"
            onClick={() => onGrant(false, false)}
            className="btn-danger"
          >
            Deny
          </button>
        </div>
      </div>
    </div>
  );
}
