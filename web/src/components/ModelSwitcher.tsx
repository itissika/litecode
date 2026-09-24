import { useMemo } from "react";

import type { ModelInfo } from "../api/types";
import { useSessionStore } from "../stores/sessionStore";
import {
  Dropdown,
  dropdownItemClass,
  dropdownItemActiveClass,
} from "./ui/Dropdown";
import { ProviderLogo } from "./ProviderLogos";

const CTRL_H = "h-7";
const CTRL_TEXT = "text-[11px]";
const PRESS =
  "transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90 disabled:pointer-events-none disabled:opacity-40 disabled:active:scale-100";
function triggerBase(open: boolean): string {
  return `${CTRL_H} ${CTRL_TEXT} ${PRESS} box-border inline-flex w-auto cursor-pointer items-center rounded-md border border-transparent px-2 leading-none text-left text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-ix-fg-hover) ${
    open ? "bg-(--_dk-ix-bg-hover)" : "bg-transparent"
  }`;
}

export interface ProviderModelGroup {
  providerId: string;
  models: ModelInfo[];
}

/** Group active models by provider (catalog order within a provider preserved). */
export function groupModelsByProvider(
  models: ModelInfo[],
): ProviderModelGroup[] {
  const groups = new Map<string, ModelInfo[]>();
  for (const model of models) {
    const bucket = groups.get(model.provider_id);
    if (bucket) bucket.push(model);
    else groups.set(model.provider_id, [model]);
  }
  return [...groups.entries()].map(([providerId, list]) => ({
    providerId,
    models: [...list].sort((a, b) =>
      (a.label?.trim() || a.api_model_id).localeCompare(
        b.label?.trim() || b.api_model_id,
      ),
    ),
  }));
}

function modelLabel(model: ModelInfo): string {
  return model.label?.trim() || model.api_model_id;
}

/**
 * Session model picker. Values are stable composite refs
 * (`{provider_id}/{model_id}`) — the exact string sessions store, never a
 * display id. A ref that is not in the active catalog is surfaced as
 * "Missing: …" instead of being silently swapped for another model.
 */
export function ModelSwitcher({
  sessionId,
  disabled = false,
  modelId: controlledModelId,
  onChange,
}: {
  sessionId: string;
  disabled?: boolean;
  modelId?: string | null;
  onChange?: (modelId: string) => void;
}) {
  const availableModels = useSessionStore((s) => s.availableModels);
  const sessionSlice = useSessionStore((s) => s.byId.get(sessionId));
  const modelId =
    controlledModelId === undefined
      ? (sessionSlice?.modelId ?? null)
      : controlledModelId;
  const label = sessionSlice?.label ?? "";
  const setModel = useSessionStore((s) => s.setModel);

  const groups = useMemo(
    () => groupModelsByProvider(availableModels),
    [availableModels],
  );
  const currentModelInfo = modelId
    ? availableModels.find((m) => m.id === modelId)
    : undefined;
  /** Set only when the session points at a ref the active catalog doesn't know. */
  const missingRef = modelId && !currentModelInfo ? modelId : null;
  const displayLabel = currentModelInfo
    ? modelLabel(currentModelInfo) || label.trim() || modelId || ""
    : "";
  const triggerText = missingRef ? `Missing: ${missingRef}` : displayLabel;

  if (availableModels.length === 0 && !missingRef) {
    return (
      <button
        type="button"
        disabled
        className={`${triggerBase(false)} min-w-[90px] max-w-[220px] shrink text-(--_dk-text-disabled)`}
        title="Configure a provider API key in Settings → Providers"
      >
        Configure a provider API key
      </button>
    );
  }

  return (
    <Dropdown
      direction="up"
      variant="select"
      className="min-w-[90px] max-w-[220px] shrink"
      panelClassName="rounded-md"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          disabled={disabled}
          onClick={toggle}
          className={`${triggerBase(open)} w-full min-w-0 disabled:cursor-not-allowed ${
            triggerText
              ? missingRef
                ? "text-(--_dk-amber-500)"
                : "text-(--_dk-text-muted)"
              : "text-(--_dk-accent-hover)"
          }`}
          title={triggerText ? `Model: ${triggerText}` : "Select model"}
        >
          <span className="flex min-w-0 items-center gap-1.5">
            <ProviderLogo providerId={currentModelInfo?.provider_id} />
            <span className="truncate">{triggerText || "Select model"}</span>
          </span>
        </button>
      )}
    >
      {groups.map((group) => (
        <div key={group.providerId}>
          <p className="px-3 pb-0.5 pt-2 text-[10px] uppercase tracking-wide text-(--_dk-text-disabled)">
            {group.providerId}
          </p>
          {group.models.map((m) => {
            const isActive = modelId != null && m.id === modelId;
            return (
              <button
                key={m.id}
                type="button"
                onClick={() => {
                  if (onChange) onChange(m.id);
                  else setModel(sessionId, m.id);
                }}
                className={`${dropdownItemClass} ${PRESS} ${isActive ? dropdownItemActiveClass : ""}`}
              >
                <span className="flex items-center gap-1.5 truncate">
                  <ProviderLogo providerId={m.provider_id} />
                  {modelLabel(m)}
                </span>
              </button>
            );
          })}
        </div>
      ))}
      {missingRef ? (
        <button
          type="button"
          disabled
          className={`${dropdownItemClass} cursor-default text-(--_dk-amber-500)`}
          title="This model is not in the active catalog — pick a model to replace it"
        >
          Missing: {missingRef}
        </button>
      ) : null}
      {availableModels.length === 0 ? (
        <p className="px-3 py-2 text-[11px] text-(--_dk-text-disabled)">
          No active models — configure a provider API key in Settings.
        </p>
      ) : null}
    </Dropdown>
  );
}
