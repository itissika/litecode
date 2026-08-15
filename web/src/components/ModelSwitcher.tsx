import { useSessionStore } from "../stores/sessionStore";
import { Dropdown, dropdownItemClass, dropdownItemActiveClass } from "./ui/Dropdown";
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

export function ModelSwitcher({
  sessionId,
  disabled = false,
}: {
  sessionId: string;
  disabled?: boolean;
}) {
  const availableModels = useSessionStore((s) => s.availableModels);
  const sessionSlice = useSessionStore((s) => s.byId.get(sessionId));
  const modelId = sessionSlice?.modelId ?? null;
  const label = sessionSlice?.label ?? "";
  const setModel = useSessionStore((s) => s.setModel);

  const currentModelInfo = modelId
    ? availableModels.find((m) => m.id === modelId)
    : undefined;
  const displayLabel = modelId
    ? currentModelInfo?.label?.trim() ||
      currentModelInfo?.api_model_id ||
      label.trim() ||
      modelId
    : "";

  if (availableModels.length === 0) {
    return (
      <button
        type="button"
        disabled
        className={`${triggerBase(false)} text-(--_dk-text-disabled)`}
        title="Add a model in Settings first"
      >
        No models
      </button>
    );
  }

  return (
    <Dropdown
      direction="up"
      variant="select"
      className="shrink-0"
      panelClassName="rounded-md"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          disabled={disabled}
          onClick={toggle}
          className={`${triggerBase(open)} disabled:cursor-not-allowed ${
            displayLabel
              ? "text-(--_dk-text-muted)"
              : "text-(--_dk-accent-hover)"
          }`}
          title={displayLabel ? `Model: ${displayLabel}` : "Select model"}
        >
          <span className="flex items-center gap-1.5">
            <ProviderLogo adapterId={currentModelInfo?.adapter_id} />
            {displayLabel || "Select model"}
          </span>
        </button>
      )}
    >
      {availableModels.map((m) => {
        const isActive = modelId != null && m.id === modelId;
        return (
          <button
            key={m.id}
            type="button"
            onClick={() => {
              setModel(sessionId, m.id);
            }}
            className={`${dropdownItemClass} ${PRESS} ${isActive ? dropdownItemActiveClass : ""}`}
          >
            <span className="flex items-center gap-1.5 truncate">
              <ProviderLogo adapterId={m.adapter_id} />
              {m.label?.trim() || m.api_model_id}
            </span>
          </button>
        );
      })}
    </Dropdown>
  );
}
