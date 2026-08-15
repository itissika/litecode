import { create } from "zustand";

export type ToastVariant = "error" | "success" | "info";

export interface Toast {
  id: string;
  message: string;
  variant: ToastVariant;
}

interface ToastStore {
  toasts: Toast[];
  showToast: (message: string, variant?: ToastVariant, durationMs?: number) => void;
  dismissToast: (id: string) => void;
}

let nextToastId = 0;

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],

  showToast: (message, variant = "error", durationMs = 5000) => {
    const id = `toast-${++nextToastId}-${Date.now()}`;
    set((s) => ({ toasts: [...s.toasts, { id, message, variant }] }));
    window.setTimeout(() => get().dismissToast(id), durationMs);
  },

  dismissToast: (id) =>
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));
