import { create } from "zustand";

export type ToastVariant = "error" | "success" | "info";

export interface Toast {
  id: string;
  message: string;
  variant: ToastVariant;
  /** Optional emoji that replaces the variant dot (personality on onboarding notes). */
  icon?: string;
  /** How many identical notes folded into this row. */
  count: number;
}

interface ToastStore {
  toasts: Toast[];
  /**
   * True while the pointer rests on the toast stack. Every auto-dismiss timer
   * is held, so a note the human is still reading never disappears mid-read.
   */
  paused: boolean;
  showToast: (
    message: string,
    variant?: ToastVariant,
    durationMs?: number,
    channel?: string,
    opts?: { icon?: string },
  ) => void;
  dismissToast: (id: string) => void;
  /** Holds (true) or resumes (false) every auto-dismiss timer at once. */
  setPaused: (paused: boolean) => void;
}

interface TimerState {
  handle: number;
  /** Time left when the timer was armed. */
  remaining: number;
  armedAt: number;
}

let nextToastId = 0;
const timers = new Map<string, TimerState>();
/** Time each toast had left when the stack was paused; no timer runs then. */
const heldMs = new Map<string, number>();

function clearTimer(id: string): void {
  const timer = timers.get(id);
  if (timer === undefined) return;
  window.clearTimeout(timer.handle);
  timers.delete(id);
}

/** Time left on a toast's timer, measured from the moment it was last armed. */
function remainingOf(id: string): number {
  const timer = timers.get(id);
  if (timer === undefined) return 0;
  return Math.max(0, timer.remaining - (Date.now() - timer.armedAt));
}

/**
 * The row an identical note folds into: a channel only folds onto its own slot,
 * a free note folds onto any row carrying the exact same text.
 */
function findDuplicate(
  toasts: Toast[],
  message: string,
  variant: ToastVariant,
  icon: string | undefined,
  channel: string | undefined,
): Toast | undefined {
  const same = (t: Toast) =>
    t.message === message && t.variant === variant && t.icon === icon;
  if (channel === undefined) return toasts.find(same);
  const slot = toasts.find((t) => t.id === channel);
  return slot !== undefined && same(slot) ? slot : undefined;
}

function armTimer(id: string, ms: number): void {
  clearTimer(id);
  heldMs.delete(id);
  const handle = window.setTimeout(() => {
    timers.delete(id);
    useToastStore.getState().dismissToast(id);
  }, ms);
  timers.set(id, { handle, remaining: ms, armedAt: Date.now() });
}

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],
  paused: false,

  showToast: (message, variant = "error", durationMs = 5000, channel, opts) => {
    const icon = opts?.icon;

    // Identical notes fold into one row instead of stacking copies; the row's
    // timer restarts so the human gets the full reading time again.
    const duplicate = findDuplicate(get().toasts, message, variant, icon, channel);
    if (duplicate !== undefined) {
      set((s) => ({
        toasts: s.toasts.map((t) =>
          t.id === duplicate.id ? { ...t, count: t.count + 1 } : t,
        ),
      }));
      if (get().paused) heldMs.set(duplicate.id, durationMs);
      else armTimer(duplicate.id, durationMs);
      return;
    }

    // A channel is a slot: raising it again replaces that slot instead of stacking.
    const id = channel ?? `toast-${++nextToastId}-${Date.now()}`;
    set((s) => ({
      toasts: [
        ...s.toasts.filter((t) => t.id !== id),
        { id, message, variant, icon, count: 1 },
      ],
    }));
    if (get().paused) heldMs.set(id, durationMs);
    else armTimer(id, durationMs);
  },

  dismissToast: (id) => {
    clearTimer(id);
    heldMs.delete(id);
    const toasts = get().toasts.filter((t) => t.id !== id);
    // An empty stack cannot be hovered: release the hold so the next note
    // dismisses normally even if the pointer never moved again.
    set(toasts.length === 0 ? { toasts, paused: false } : { toasts });
    if (toasts.length === 0) heldMs.clear();
  },

  setPaused: (paused) => {
    if (get().paused === paused) return;
    if (paused) {
      for (const id of timers.keys()) heldMs.set(id, remainingOf(id));
      for (const id of [...timers.keys()]) clearTimer(id);
      set({ paused: true });
      return;
    }
    set({ paused: false });
    const held = [...heldMs];
    heldMs.clear();
    for (const [id, ms] of held) armTimer(id, ms);
  },
}));
