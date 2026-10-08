import { MagnifyingGlass } from "@phosphor-icons/react";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";

import type { ModelInfo } from "../api/types";
import { paintDragCursor, viewOf } from "../lib/domView";
import { splitModelRef } from "../api/settings";
import { useSessionStore } from "../stores/sessionStore";
import { useSettingsStore } from "../stores/settingsStore";
import {
  Dropdown,
  dropdownItemClass,
  dropdownItemActiveClass,
} from "./ui/Dropdown";
import { motion, useReducedMotion } from "motion/react";
import { glassFill } from "./composerCard";
import { ModalityIcons } from "./ModalityIcons";
import { ProviderLogo } from "./ProviderLogos";

const CTRL_H = "h-7";
const CTRL_TEXT = "text-[11px]";
const PRESS =
  "transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90 disabled:pointer-events-none disabled:opacity-40 disabled:active:scale-100";
function triggerBase(open: boolean, compact = false): string {
  const spacing = compact
    ? "overflow-hidden border-0 px-2"
    : "border border-transparent px-2";
  return `${CTRL_H} ${CTRL_TEXT} ${PRESS} box-border inline-flex w-auto cursor-pointer items-center rounded-md ${spacing} leading-none text-left text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-ix-fg-hover) ${
    open ? "bg-(--_dk-ix-bg-hover)" : "bg-transparent"
  }`;
}

/** Settings form control: same underline as `Select`, not the composer chip. */
const FIELD_TRIGGER =
  "flex w-full min-w-0 items-center justify-between gap-1 border-0 border-b border-(--_dk-line) bg-transparent px-1 py-[0.375rem] text-left text-[0.875rem] text-(--_dk-text-muted) hover:brightness-110 focus-visible:border-(--_dk-line-visible) disabled:cursor-not-allowed disabled:opacity-40";

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="shrink-0 text-(--_dk-text-disabled) transition-transform duration-150"
      style={{ transform: open ? "rotate(180deg)" : "rotate(0deg)" }}
    >
      <path d="M3 1.5l4 3.5-4 3.5" />
    </svg>
  );
}

interface ProviderModelGroup {
  providerId: string;
  models: ModelInfo[];
}

/** Group active models by provider (catalog order within a provider preserved). */
function groupModelsByProvider(models: ModelInfo[]): ProviderModelGroup[] {
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

/** Floor for a dragged list height — header plus one readable row.
 *  Not applied when the open side is shorter than this. */
const MIN_PANEL_H = 96;

/** Content column whose ceiling is the height Dropdown already resolved.
 *  Pins to that ceiling only when the rows are taller, so the list scrolls
 *  and the filter stays put. */
function CappedColumn({
  bodyRef,
  maxHeight,
  height,
  children,
}: {
  bodyRef: RefObject<HTMLDivElement | null>;
  maxHeight: number;
  height?: number;
  children: ReactNode;
}) {
  const [pin, setPin] = useState(false);
  useLayoutEffect(() => {
    const el = bodyRef.current;
    if (!el || height != null) {
      setPin(false);
      return;
    }
    const next = el.scrollHeight > maxHeight + 1;
    setPin((current) => (current === next ? current : next));
  }, [bodyRef, maxHeight, height]);
  return (
    <div
      ref={bodyRef}
      className="flex min-h-0 flex-col overflow-hidden"
      style={{
        maxHeight,
        height: height ?? (pin ? maxHeight : undefined),
      }}
    >
      {children}
    </div>
  );
}

function matchesQuery(model: ModelInfo, query: string): boolean {
  if (!query) return true;
  return (
    modelLabel(model).toLowerCase().includes(query) ||
    model.api_model_id.toLowerCase().includes(query)
  );
}

/**
 * Model picker shared by the composer toolbar and Settings → Agents.
 *
 * Values are stable composite refs (`{provider_id}/{model_id}`) — the exact
 * string sessions and agent profiles store, never a display id. A ref that is
 * not in the active catalog is surfaced as "Missing: …" instead of being
 * silently swapped for another model.
 *
 * `composer` is the compact glass chip that opens upward from the chat input.
 * `field` is the settings underline control: overlay menu, opens downward,
 * same list (search, provider filter, modalities) underneath.
 */
export function ModelSwitcher({
  sessionId,
  disabled = false,
  modelId: controlledModelId,
  onChange,
  models,
  variant = "composer",
}: {
  sessionId?: string;
  disabled?: boolean;
  modelId?: string | null;
  onChange?: (modelId: string) => void;
  /** Active catalog. Omit to use the session handshake list. */
  models?: ModelInfo[];
  variant?: "composer" | "field";
}) {
  const field = variant === "field";
  const sessionModels = useSessionStore((s) => s.availableModels);
  const availableModels = models ?? sessionModels;
  const sessionSlice = useSessionStore((s) =>
    sessionId ? s.byId.get(sessionId) : undefined,
  );
  const modelId =
    controlledModelId === undefined
      ? (sessionSlice?.modelId ?? null)
      : controlledModelId;
  const label = sessionSlice?.label ?? "";
  const setModel = useSessionStore((s) => s.setModel);
  const llmSettings = useSettingsStore((s) => s.llm);
  const reduceMotion = useReducedMotion();
  // Reordering animates position only: a row never changes size, and a size
  // projection would re-animate on every frame of the drag-resize.
  const reorder = reduceMotion
    ? { duration: 0 }
    : { type: "spring" as const, stiffness: 420, damping: 34 };
  const [modelQuery, setModelQuery] = useState("");
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(
    null,
  );

  const groups = useMemo(
    () => groupModelsByProvider(availableModels),
    [availableModels],
  );
  const providerNames = useMemo(
    () =>
      new Map(
        (llmSettings?.providers ?? []).map((provider) => [
          provider.id,
          provider.name,
        ] as const),
      ),
    [llmSettings],
  );
  const hasProviderFilter =
    selectedProviderId != null &&
    groups.some((group) => group.providerId === selectedProviderId);
  // Filtering reorders and dims instead of removing: nothing ever leaves the
  // list, so the panel keeps its height while the query is typed. Selected
  // provider first, then providers that still hold a hit, then the rest.
  const orderedGroups = useMemo(() => {
    const query = modelQuery.trim().toLowerCase();
    const rows = groups.map((group, order) => {
      const scored = group.models.map((model) => ({
        model,
        matched: matchesQuery(model, query),
      }));
      const hit = query.length > 0 && scored.some((entry) => entry.matched);
      const selected = group.providerId === selectedProviderId;
      // Nothing dims while no filter is active; `hit` is already false for an
      // empty query, so the no-filter case has to rank 0 explicitly.
      const rank = selectedProviderId
        ? selected
          ? 0
          : hit
            ? 1
            : 2
        : query.length > 0
          ? hit
            ? 0
            : 1
          : 0;
      scored.sort((a, b) => Number(b.matched) - Number(a.matched));
      return {
        providerId: group.providerId,
        dim: rank > 0,
        models: scored.map((entry) => ({
          model: entry.model,
          dim: rank > 0 || !entry.matched,
        })),
        rank,
        order,
      };
    });
    return rows.sort((a, b) => a.rank - b.rank || a.order - b.order);
  }, [groups, modelQuery, selectedProviderId]);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  /** Ceiling Dropdown resolved for the open menu — the drag cannot pass it. */
  const capRef = useRef(MIN_PANEL_H);
  const draggingRef = useRef(false);
  const dragStartRef = useRef({ y: 0, h: 0, max: MIN_PANEL_H });
  /** Last height written by the live drag — committed to state on pointer up,
   *  so no layout read is needed at the end of the gesture. */
  const dragHeightRef = useRef<number | null>(null);
  const rafRef = useRef<number | null>(null);
  const rafViewRef = useRef<Window>(window);
  /** Height written by a drag; `null` means "size to content". */
  const [listHeight, setListHeight] = useState<number | null>(null);

  const cancelDragFrame = () => {
    if (rafRef.current == null) return;
    rafViewRef.current.cancelAnimationFrame(rafRef.current);
    rafRef.current = null;
  };

  useEffect(() => {
    return () => {
      cancelDragFrame();
      // A drag cut short by the panel closing (Escape, outside click) must not
      // leave the page stuck in its resize cursor.
      if (draggingRef.current) {
        draggingRef.current = false;
        paintDragCursor(bodyRef.current, false);
      }
    };
  }, []);

  const applyResize = (clientY: number) => {
    rafRef.current = null;
    const body = bodyRef.current;
    if (!body) return;
    // The panel opens upward, so dragging the grip up grows the list.
    const delta = dragStartRef.current.y - clientY;
    const cap = dragStartRef.current.max;
    const floor = Math.min(MIN_PANEL_H, cap);
    const height = Math.max(
      floor,
      Math.min(cap, dragStartRef.current.h + delta),
    );
    dragHeightRef.current = Math.round(height);
    body.style.height = `${height}px`;
  };

  // Pointer capture, same as the composer's handle: every move/up for this
  // pointer keeps arriving at the grip even if the cursor leaves it.
  const onResizeStart = (event: React.PointerEvent) => {
    event.preventDefault();
    const body = bodyRef.current;
    if (!body) return;
    draggingRef.current = true;
    const rect = body.getBoundingClientRect();
    dragStartRef.current = {
      y: event.clientY,
      h: rect.height,
      max: capRef.current,
    };
    dragHeightRef.current = Math.round(rect.height);
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
    paintDragCursor(event.currentTarget, true);
  };

  const onResizeEnd = (event: React.PointerEvent) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    cancelDragFrame();
    const grip = event.currentTarget as HTMLElement;
    if (grip.hasPointerCapture?.(event.pointerId))
      grip.releasePointerCapture(event.pointerId);
    paintDragCursor(grip, false);
    if (dragHeightRef.current != null) setListHeight(dragHeightRef.current);
  };

  const onResizeMove = (event: React.PointerEvent) => {
    if (!draggingRef.current) return;
    // Coalesce high-frequency moves into one style write per frame.
    const y = event.clientY;
    cancelDragFrame();
    const view = viewOf(event.currentTarget);
    rafViewRef.current = view;
    rafRef.current = view.requestAnimationFrame(() => applyResize(y));
  };

  const currentModelInfo = modelId
    ? availableModels.find((m) => m.id === modelId)
    : undefined;
  /** Set only when the value points at a ref the active catalog doesn't know. */
  const missingRef = modelId && !currentModelInfo ? modelId : null;
  const missingProviderId = missingRef
    ? splitModelRef(missingRef).providerId || undefined
    : undefined;
  const displayLabel = currentModelInfo
    ? modelLabel(currentModelInfo) || label.trim() || modelId || ""
    : "";
  const triggerText = missingRef ? `Missing: ${missingRef}` : displayLabel;
  const logoProviderId = currentModelInfo?.provider_id ?? missingProviderId;

  if (availableModels.length === 0 && !missingRef) {
    if (field) {
      return (
        <button type="button" disabled className={FIELD_TRIGGER}>
          <span className="truncate text-(--_dk-text-disabled)">
            Select model
          </span>
          <Chevron open={false} />
        </button>
      );
    }
    return (
      <button
        type="button"
        disabled
        className={`${triggerBase(false)} min-w-3.5 max-w-[220px] shrink text-(--_dk-text-disabled)`}
        title="Configure a provider API key in Settings → Providers"
      >
        Configure a provider API key
      </button>
    );
  }

  return (
    <Dropdown
      direction={field ? "down" : "up"}
      variant="select"
      // No fixed ceiling. Dropdown keeps the list inside the screen and the
      // pane, and may still open the other way when that side is the taller one.
      maxHeight={null}
      className={
        field ? "w-full min-w-0" : "min-w-3.5 max-w-[220px] shrink"
      }
      // Composer: frosted fill, because the panel is portaled to `body` and
      // would otherwise lose the card it opens from. Settings: the overlay
      // menu every other form select uses. Border and shadow stay the menu's.
      bgClassName={field ? undefined : glassFill}
      panelClassName={field ? "rounded-md" : "w-60 rounded-md"}
      trigger={({ open, toggle }) =>
        field ? (
          <button
            type="button"
            disabled={disabled}
            onClick={toggle}
            className={FIELD_TRIGGER}
            title={triggerText ? `Model: ${triggerText}` : "Select model"}
          >
            <span
              className={`flex min-w-0 items-center gap-1.5 ${
                missingRef
                  ? "text-(--_dk-amber-500)"
                  : triggerText
                    ? "text-(--_dk-text-primary)"
                    : "text-(--_dk-text-disabled)"
              }`}
            >
              <ProviderLogo providerId={logoProviderId} />
              <span className="truncate">{triggerText || "Select model"}</span>
              <ModalityIcons modalities={currentModelInfo?.modalities} />
            </span>
            <Chevron open={open} />
          </button>
        ) : (
          <button
            type="button"
            disabled={disabled}
            onClick={toggle}
            className={`${triggerBase(open, true)} w-full min-w-3.5 disabled:cursor-not-allowed ${
              triggerText
                ? missingRef
                  ? "text-(--_dk-amber-500)"
                  : "text-(--_dk-text-muted)"
                : "text-(--_dk-accent-hover)"
            }`}
            title={triggerText ? `Model: ${triggerText}` : "Select model"}
          >
            <span className="flex min-w-0 items-center gap-1.5">
              <ProviderLogo providerId={logoProviderId} />
              <span className="truncate">{triggerText || "Select model"}</span>
              {/* What the current model takes beyond text — the same glyphs as
                  Settings → Models, so the composer answers "can this model see
                  my image?" without opening the list. */}
              <ModalityIcons modalities={currentModelInfo?.modalities} />
            </span>
          </button>
        )
      }
    >
      {({ maxHeight }) => {
        capRef.current = maxHeight;
        return (
          <CappedColumn
            bodyRef={bodyRef}
            maxHeight={maxHeight}
            height={
              listHeight != null ? Math.min(listHeight, maxHeight) : undefined
            }
          >
        {availableModels.length > 0 ? (
          <div
            className="flex shrink-0 items-center gap-1 px-2 pt-2"
            onClick={(event) => event.stopPropagation()}
          >
            <div
              className="flex h-7 min-w-0 flex-1 items-center gap-1 rounded border border-(--_dk-line) bg-(--_dk-editor) px-1.5"
              role="search"
            >
              <MagnifyingGlass
                size={12}
                className="shrink-0 text-(--_dk-text-muted)"
              />
              <input
                aria-label="Filter models"
                value={modelQuery}
                onChange={(event) => setModelQuery(event.target.value)}
                placeholder="Filter models"
                className="min-w-0 flex-1 border-0 bg-transparent py-0.5 text-[11px] text-(--_dk-text-secondary) outline-none placeholder:text-(--_dk-text-disabled)"
              />
            </div>
            <div
              className="flex h-4 w-6 shrink-0 cursor-ns-resize items-center justify-center rounded text-(--_dk-text-disabled) select-none hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary)"
              style={{ touchAction: "none" }}
              title="Drag to resize the list"
              onPointerDown={onResizeStart}
              onPointerMove={onResizeMove}
              onPointerUp={onResizeEnd}
              onPointerCancel={onResizeEnd}
            >
              <svg
                width="10"
                height="10"
                viewBox="0 0 10 10"
                fill="currentColor"
              >
                <circle cx="2" cy="2" r="1" />
                <circle cx="5" cy="2" r="1" />
                <circle cx="8" cy="2" r="1" />
              </svg>
            </div>
          </div>
        ) : null}
        {availableModels.length > 0 ? (
          <div
            role="group"
            aria-label="Filter by provider"
            className="flex shrink-0 flex-wrap gap-1 px-2 py-1"
            onClick={(event) => event.stopPropagation()}
          >
            {groups.map((group) => {
              const selected = selectedProviderId === group.providerId;
              const providerName =
                providerNames.get(group.providerId) ?? group.providerId;
              return (
                <button
                  key={group.providerId}
                  type="button"
                  aria-label={`Filter models from ${providerName}`}
                  aria-pressed={selected}
                  title={`Filter models from ${providerName}`}
                  onClick={() =>
                    setSelectedProviderId((current) =>
                      current === group.providerId ? null : group.providerId,
                    )
                  }
                  className={`${PRESS} flex h-6 w-6 items-center justify-center rounded-md ${
                    selected
                      ? "bg-(--_dk-ix-bg-hover) text-(--_dk-ix-fg-selected)"
                      : hasProviderFilter
                        ? "text-(--_dk-text-muted) opacity-40"
                        : "text-(--_dk-text-muted)"
                  }`}
                >
                  <ProviderLogo
                    providerId={group.providerId}
                    title={providerName}
                  />
                </button>
              );
            })}
          </div>
        ) : null}
        <motion.div layoutScroll className="min-h-0 grow overflow-y-auto pb-1">
          {orderedGroups.map((row) => (
            <motion.div
              key={row.providerId}
              layout="position"
              transition={reorder}
            >
              <p
                className={`px-3 pb-0.5 pt-2 text-[10px] uppercase tracking-wide text-(--_dk-text-disabled) ${
                  row.dim ? "opacity-40" : ""
                }`}
              >
                {providerNames.get(row.providerId) ?? row.providerId}
              </p>
              {row.models.map(({ model: m, dim }) => {
                const isActive = modelId != null && m.id === modelId;
                return (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => {
                      if (onChange) onChange(m.id);
                      else if (sessionId) setModel(sessionId, m.id);
                    }}
                    className={`${dropdownItemClass} group ${
                      isActive ? dropdownItemActiveClass : ""
                    } ${dim ? "opacity-40" : ""}`}
                  >
                    <span className="flex items-center gap-1.5">
                      {/* The row owns the gesture, so the icon answers to
                          group-hover/group-active rather than its own. Negative
                          margin cancels the chip padding: the highlight can
                          bleed past the glyph without shifting the label. */}
                      <span
                        className={`-m-0.5 inline-flex shrink-0 items-center justify-center rounded p-0.5 transition-all duration-100 group-hover:scale-120 group-hover:brightness-125 group-active:scale-90 group-active:brightness-90 ${
                          isActive
                            ? "bg-[color-mix(in_srgb,var(--_dk-accent-hover)_14%,transparent)]"
                            : ""
                        }`}
                      >
                        <ProviderLogo providerId={m.provider_id} />
                      </span>
                      <span className="min-w-0 truncate">{modelLabel(m)}</span>
                      <ModalityIcons modalities={m.modalities} />
                    </span>
                  </button>
                );
              })}
            </motion.div>
          ))}
          {missingRef ? (
            <button
              type="button"
              disabled
              className={`${dropdownItemClass} cursor-default text-(--_dk-amber-500)`}
              title="This model is not in the active catalog — pick a model to replace it"
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <ProviderLogo providerId={missingProviderId} />
                <span className="truncate">Missing: {missingRef}</span>
              </span>
            </button>
          ) : null}
          {availableModels.length === 0 ? (
            <p className="px-3 py-2 text-[11px] text-(--_dk-text-disabled)">
              No active models — configure a provider API key in Settings.
            </p>
          ) : null}
        </motion.div>
          </CappedColumn>
        );
      }}
    </Dropdown>
  );
}
