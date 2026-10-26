import type { ReactNode } from "react";

/** One width for every usage tooltip, so a hover reads the same everywhere. */
export const TOOLTIP_W = 138;

/** Bubble position: centred on the column, clamped inside the chart. */
export function tooltipLeft(x: number, chartWidth: number): number {
  return Math.min(Math.max(0, x - TOOLTIP_W / 2), chartWidth - TOOLTIP_W);
}

/**
 * The hover bubble both usage charts share.
 *
 * Native `<title>` waits about a second and wears the OS theme; a chart that
 * answers on mouse-down needs to answer on mouse-move.
 */
export function ChartTooltip({
  left,
  bottom,
  children,
}: {
  left: number;
  bottom: number;
  children: ReactNode;
}) {
  return (
    <div
      className="pointer-events-none absolute rounded border border-(--_dk-line-visible) bg-(--_dk-overlay) px-1.5 py-1 text-[10px] leading-snug shadow-[0_4px_12px_rgba(0,0,0,0.18)]"
      style={{ width: TOOLTIP_W, left, bottom }}
    >
      {children}
    </div>
  );
}

/** `label · value`, monospaced so the numbers line up between hovers. */
export function TooltipRow({
  label,
  value,
  muted = false,
}: {
  label: ReactNode;
  value: ReactNode;
  muted?: boolean;
}) {
  return (
    <div
      className={`flex justify-between font-mono tabular-nums ${
        muted ? "text-(--_dk-text-muted)" : "text-(--_dk-text-secondary)"
      }`}
    >
      <span>{label}</span>
      <span>{value}</span>
    </div>
  );
}
