import { useMemo, useState } from "react";
import {
  FilePdfIcon,
  ImageIcon,
  SpeakerHighIcon,
  VideoIcon,
} from "@phosphor-icons/react";
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
 * Input modalities as 12px glyphs, in the catalog's own order
 * (`Modality::ALL` in Rust, minus `text`) so every row scans the same way.
 *
 * `text` is deliberately absent: every model must declare it
 * (`resolve_model` rejects a catalog without it), so a text glyph would sit on
 * every row identically and say nothing. The glyphs mark what a model takes
 * *beyond* text, and a row with none stays name plus switch.
 *
 * The wire sends a closed set of tokens, so one the table does not name is
 * dropped rather than given a guess of an icon.
 */
const MODALITY_GLYPHS: { token: string; Glyph: typeof ImageIcon }[] = [
  { token: "image", Glyph: ImageIcon },
  { token: "video", Glyph: VideoIcon },
  { token: "audio", Glyph: SpeakerHighIcon },
  { token: "pdf", Glyph: FilePdfIcon },
];

function ModalityIcons({ modalities }: { modalities: string[] }) {
  const present = MODALITY_GLYPHS.filter(({ token }) =>
    modalities.includes(token),
  );
  if (present.length === 0) return null;
  return (
    <span
      className="flex shrink-0 items-center gap-1 text-(--_dk-text-muted)"
      aria-label="Input modalities"
    >
      {present.map(({ token, Glyph }) => (
        <span
          key={token}
          role="img"
          aria-label={token}
          title={`Accepts ${token} input`}
          className="inline-flex"
        >
          <Glyph size={12} aria-hidden />
        </span>
      ))}
    </span>
  );
}

/**
 * Binary On/Off picker in the same segmented style as the chat input's
 * context/thinking switches: the active segment is a motion pill that springs
 * between the two slots (reduced-motion users get an instant swap).
 *
 * `id` (the globally unique `provider/model` ref) keys the shared-layout pill,
 * never the display label: two providers ship the same label ("GPT-6 Luna" on
 * both OpenCode hosts), and a label-keyed `layoutId` makes one row's pill the
 * lead for every row that matches, so the duplicates render no pill of their
 * own. The ref is the identity the switch writes to the server anyway.
 */
function ModelToggle({
  id,
  label,
  enabled,
  disabled,
  onToggle,
}: {
  id: string;
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
    <LayoutGroup id={`model-toggle-${id}`}>
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
              className={`group relative flex h-6 min-w-[48px] items-center justify-center rounded border border-transparent px-2 text-dk-xs leading-none text-(--_dk-text-muted) transition-transform duration-100 hover:text-(--_dk-ix-fg-hover) hover:brightness-110 active:scale-90 active:brightness-90 disabled:pointer-events-none disabled:opacity-40 ${
                selected
                  ? "text-(--_dk-accent-hover) hover:text-(--_dk-accent-hover)"
                  : ""
              }`}
            >
              {selected ? (
                <motion.span
                  layoutId={`model-toggle-pill-${id}`}
                  data-layout-id={`model-toggle-pill-${id}`}
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
 * One model: its name, the input modalities it accepts, and whether the pickers
 * may offer it.
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
      className="flex min-w-0 items-center justify-between gap-2 py-0.5 pr-3 pl-2"
      title={
        model.tool_call
          ? undefined
          : `${label} can't call tools — agents can't run on it`
      }
    >
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="min-w-0 truncate text-dk-sm text-(--_dk-text-secondary)">
          {label}
        </span>
        <ModalityIcons modalities={model.modalities} />
      </span>
      <ModelToggle
        id={model.ref}
        label={label}
        enabled={model.enabled}
        disabled={saveBlocked || busy}
        onToggle={onToggle}
      />
    </div>
  );
}

function ProviderModels({
  provider,
  ...rest
}: {
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
      className="settings-foldcard settings-foldcard-compact"
    >
      <div
        className="settings-card overflow-hidden p-0"
        role="list"
        aria-label={`Models for ${provider.name}`}
      >
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
