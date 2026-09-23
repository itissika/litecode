import { useMemo, useState } from "react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";

import {
  modelRefLabel,
  type CatalogModelDto,
  type CatalogProviderDto,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { FoldCard } from "../../../components/FoldCard";
import { ProviderLogo } from "../../../components/ProviderLogos";
import { SettingsPageShell, useSettingsSaveBlocked } from "./shared";

/**
 * Binary On/Off picker in the same segmented style as the chat input's
 * context/thinking switches: the active segment is a motion pill that springs
 * between the two slots (reduced-motion users get an instant swap).
 */
function ModelToggle({
  label,
  enabled,
  disabled,
  onToggle,
}: {
  label: string;
  enabled: boolean;
  disabled: boolean;
  onToggle: (enabled: boolean) => void;
}) {
  const reduceMotion = useReducedMotion();
  const spring = reduceMotion
    ? { duration: 0 }
    : { type: "spring" as const, stiffness: 420, damping: 34 };
  return (
    <LayoutGroup id={`model-toggle-${label}`}>
      <div
        role="group"
        aria-label={`Enable ${label}`}
        className="flex shrink-0"
      >
        {[
          { text: "On", value: true },
          { text: "Off", value: false },
        ].map(({ text, value }) => {
          const selected = enabled === value;
          return (
            <button
              key={text}
              type="button"
              disabled={disabled}
              aria-pressed={selected}
              onClick={() => onToggle(value)}
              className={`group relative flex h-7 min-w-[48px] items-center justify-center rounded border border-transparent px-2 text-[11px] leading-none text-(--_dk-text-muted) transition-transform duration-100 hover:text-(--_dk-ix-fg-hover) hover:brightness-110 active:scale-90 active:brightness-90 disabled:pointer-events-none disabled:opacity-40 ${
                selected
                  ? "text-(--_dk-accent-hover) hover:text-(--_dk-accent-hover)"
                  : ""
              }`}
            >
              {selected ? (
                <motion.span
                  layoutId={`model-toggle-pill-${label}`}
                  className="absolute inset-0 rounded bg-(--_dk-accent-halo)"
                  transition={spring}
                />
              ) : (
                <span className="pointer-events-none absolute inset-0 rounded border border-transparent group-hover:border-(--_dk-line)" />
              )}
              <span className="relative z-10">{text}</span>
            </button>
          );
        })}
      </div>
    </LayoutGroup>
  );
}

/**
 * One model: its name and whether the pickers may offer it. Nothing else.
 *
 * `tool_call: false` models would break an agent run, so the reason rides on the
 * row's tooltip rather than on a badge — the list stays two columns wide.
 */
function ModelRow({
  model,
  busy,
  saveBlocked,
  onToggle,
}: {
  model: CatalogModelDto;
  busy: boolean;
  saveBlocked: boolean;
  onToggle: (enabled: boolean) => void;
}) {
  const label = modelRefLabel(model);
  return (
    <div
      role="listitem"
      className="flex min-w-0 items-center justify-between gap-2 px-3 py-1.5"
      title={
        model.tool_call
          ? undefined
          : `${label} can't call tools — agents can't run on it`
      }
    >
      <span className="min-w-0 truncate text-sm text-(--_dk-text-secondary)">
        {label}
      </span>
      <ModelToggle
        label={label}
        enabled={model.enabled}
        disabled={saveBlocked || busy}
        onToggle={onToggle}
      />
    </div>
  );
}

function ProviderModels({ provider, ...rest }: {
  provider: CatalogProviderDto;
  busyRef: string | null;
  saveBlocked: boolean;
  onToggle: (modelRef: string, enabled: boolean) => void;
}) {
  return (
    <FoldCard
      defaultOpen
      label={
        <span className="flex min-w-0 flex-1 items-center gap-1.5">
          <ProviderLogo providerId={provider.id} />
          <span className="truncate text-dk-lg text-(--_dk-text-primary)">
            {provider.name}
          </span>
        </span>
      }
      className="settings-foldcard"
    >
      <div className="settings-card overflow-hidden p-0" role="list" aria-label={`Models for ${provider.name}`}>
        {provider.models.map((model) => (
          <ModelRow
            key={model.ref}
            model={model}
            busy={rest.busyRef === model.ref}
            saveBlocked={rest.saveBlocked}
            onToggle={(enabled) => rest.onToggle(model.ref, enabled)}
          />
        ))}
      </div>
    </FoldCard>
  );
}

/**
 * Models page: one fold card per configured provider, one switch per model.
 *
 * A provider with no credential has no usable models, so it is absent here —
 * paste its key on the Providers page and its models appear.
 */
export function ModelsSection() {
  const llm = useSettingsStore((s) => s.llm);
  const setModelEnabled = useSettingsStore((s) => s.setModelEnabled);
  const saveBlocked = useSettingsSaveBlocked();
  const [busyRef, setBusyRef] = useState<string | null>(null);
  /**
   * Checkbox position while the write is in flight. Cleared once the store has
   * the server's answer, so the projection is always the source of truth.
   */
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});

  const providers = useMemo(
    () =>
      (llm?.providers ?? [])
        .filter((provider) => provider.visible && provider.configured)
        .filter((provider) => provider.models.length > 0)
        .map((provider) => ({
          ...provider,
          models: provider.models.map((model) => ({
            ...model,
            enabled: overrides[model.ref] ?? model.enabled,
          })),
        })),
    [llm, overrides],
  );

  const toggle = (modelRef: string, enabled: boolean) => {
    setOverrides((prev) => ({ ...prev, [modelRef]: enabled }));
    setBusyRef(modelRef);
    void (async () => {
      try {
        await setModelEnabled(modelRef, enabled);
      } catch {
        // Store toasted; dropping the override snaps the box back to the
        // projection the server still holds.
      } finally {
        setOverrides((prev) => {
          if (!(modelRef in prev)) return prev;
          const next = { ...prev };
          delete next[modelRef];
          return next;
        });
        setBusyRef(null);
      }
    })();
  };

  return (
    <SettingsPageShell title="Models">
      <div className="settings-content-indent space-y-2">
        {providers.length === 0 ? (
          <p className="text-sm text-(--_dk-text-disabled)">
            No provider holds a key yet. Add one on the Providers page — its
            models show up here.
          </p>
        ) : (
          providers.map((provider) => (
            <ProviderModels
              key={provider.id}
              provider={provider}
              busyRef={busyRef}
              saveBlocked={saveBlocked}
              onToggle={toggle}
            />
          ))
        )}
      </div>
    </SettingsPageShell>
  );
}
