import { useEffect, useState } from "react";

/** Slack over the chip's CSS exit transition (chat.css `.dock-chip`, 180ms)
 *  before the chip is unmounted. */
const EXIT_MS = 220;

/**
 * Mount-then-open animation shared by dock chips (TerminalStatusBar,
 * ActivePlanChip). Mount closed → open on the next frame so the CSS transition
 * runs; on hide animate out first and only unmount once the transition
 * finished. Returns `mounted` (keep in the DOM) and `open` (drive `.is-open`).
 */
export function useChipEntrance(visible: boolean) {
  const [mounted, setMounted] = useState(false);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (visible) {
      setMounted(true);
    } else {
      setOpen(false);
    }
  }, [visible]);

  useEffect(() => {
    if (!mounted || !visible) return;
    const raf = requestAnimationFrame(() => setOpen(true));
    return () => cancelAnimationFrame(raf);
  }, [mounted, visible]);

  useEffect(() => {
    if (visible || open || !mounted) return;
    const timer = window.setTimeout(() => setMounted(false), EXIT_MS);
    return () => window.clearTimeout(timer);
  }, [visible, open, mounted]);

  return { mounted, open };
}
