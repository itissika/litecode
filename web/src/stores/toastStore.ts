import { create } from "zustand";

export type ToastVariant = "error" | "success" | "info";

export interface Toast {
  id: string;
  message: string;
  variant: ToastVariant;
}

interface ToastStore {
  toasts: Toast[];
  showToast: (
    message: string,
    variant?: ToastVariant,
    durationMs?: number,
    channel?: string,
  ) => void;
  dismissToast: (id: string) => void;
}

let nextToastId = 0;
const channelTimers = new Map<string, number>();

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],

  showToast: (message, variant = "error", durationMs = 5000, channel) => {
    const id = channel ?? `toast-${++nextToastId}-${Date.now()}`;
    const prevTimer = channelTimers.get(id);
    if (prevTimer !== undefined) window.clearTimeout(prevTimer);
    set((s) => ({
      toasts: [...s.toasts.filter((t) => t.id !== id), { id, message, variant }],
    }));
    const timer = window.setTimeout(() => {
      channelTimers.delete(id);
      get().dismissToast(id);
    }, durationMs);
    channelTimers.set(id, timer);
  },

  dismissToast: (id) =>
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));
