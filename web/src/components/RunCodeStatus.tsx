import { HourglassIcon, WarningCircleIcon } from "@phosphor-icons/react";

import type { RunCodeChip } from "../lib/runCodeSummary";
import { FOLDCARD_HEADER_TONE } from "./FoldCard";
import { glyphFor } from "./ToolIcon";
import { WaveText } from "./WaveText";

export const RUN_CODE_CHIP_STAGGER_MS = 70;
export const RUN_CODE_CHIP_DURATION_MS = 240;
export const RUN_CODE_CHIP_GAP_MS = 60;

/** Time from the first chip to the end of the last chip (plus a small gap). */
export function runCodeChipTotalMs(count: number): number {
  if (count <= 0) return 0;
  return (
    (count - 1) * RUN_CODE_CHIP_STAGGER_MS +
    RUN_CODE_CHIP_DURATION_MS +
    RUN_CODE_CHIP_GAP_MS
  );
}

interface RunCodeStatusProps {
  chips: RunCodeChip[];
  streaming: boolean;
  hasOutput: boolean;
  /** True only for the render where the result just landed. */
  animate: boolean;
  ariaLabel?: string | null;
  fallback?: string | null;
}

/**
 * Header status for run_code: a crafting wave before the result, then the
 * aggregate tool chips. Chips never glow; the run_code glyph owns the settle
 * animation.
 */
export function RunCodeStatus({
  chips,
  streaming,
  hasOutput,
  animate,
  ariaLabel,
  fallback,
}: RunCodeStatusProps) {
  if (streaming && !hasOutput) {
    return (
      <span className={`${FOLDCARD_HEADER_TONE} min-w-0 flex-1 truncate`}>
        <WaveText text="crafting..." className="text-dk-xs" />
      </span>
    );
  }

  if (hasOutput && chips.length > 0) {
    return (
      <span
        className={`${FOLDCARD_HEADER_TONE} min-w-0 flex-1 truncate text-dk-xs`}
        role="group"
        aria-label={ariaLabel ?? undefined}
        title={ariaLabel ?? undefined}
      >
        {chips.map((chip, index) => (
          <RunCodeChipView
            key={chip.key}
            chip={chip}
            index={index}
            animate={animate}
          />
        ))}
      </span>
    );
  }

  return fallback ? (
    <span
      className={`${FOLDCARD_HEADER_TONE} min-w-0 flex-1 truncate text-(--_dk-text-muted)`}
    >
      {fallback}
    </span>
  ) : null;
}

function RunCodeChipView({
  chip,
  index,
  animate,
}: {
  chip: RunCodeChip;
  index: number;
  animate: boolean;
}) {
  const Glyph =
    chip.kind === "failed"
      ? WarningCircleIcon
      : chip.kind === "unsettled"
        ? HourglassIcon
        : glyphFor(chip.label);
  const tone =
    chip.kind === "failed"
      ? "text-(--_dk-red-500)"
      : chip.kind === "unsettled"
        ? "text-(--_dk-amber-500)"
        : "text-(--_dk-text-muted)";
  const label =
    chip.kind === "unsettled"
      ? chip.count + " unsettled"
      : chip.count > 0
        ? chip.label + " ×" + chip.count
        : chip.label;

  return (
    <span
      className={`run-code-chip mr-3 inline-flex shrink-0 items-center gap-1 align-middle whitespace-nowrap font-mono text-dk-2xs last:mr-0 ${tone} ${
        animate ? "run-code-chip-in" : ""
      }`}
      style={
        animate
          ? { animationDelay: `${index * RUN_CODE_CHIP_STAGGER_MS}ms` }
          : undefined
      }
      title={label}
    >
      <Glyph size={12} weight="fill" aria-hidden />
      {chip.kind === "unsettled" ? (
        <span>{label}</span>
      ) : (
        <>
          <span>{chip.label}</span>
          {chip.count > 0 && (
            <span className="tabular-nums">×{chip.count}</span>
          )}
        </>
      )}
    </span>
  );
}
