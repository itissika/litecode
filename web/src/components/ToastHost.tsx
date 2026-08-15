import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { useToastStore, type ToastVariant } from "../stores/toastStore";

const VARIANT_DOT: Record<ToastVariant, string> = {
  success: "bg-(--_dk-emerald-500)",
  error: "bg-(--_dk-red-500)",
  info: "bg-(--_dk-text-muted)",
};

export function ToastHost() {
  const toasts = useToastStore((s) => s.toasts);
  const dismissToast = useToastStore((s) => s.dismissToast);
  const reduceMotion = useReducedMotion();

  return (
    <div
      className="pointer-events-none fixed bottom-4 right-4 z-100 flex max-w-sm flex-col gap-2"
      aria-live="polite"
    >
      <AnimatePresence initial={false}>
        {toasts.map((toast) => (
          <motion.div
            key={toast.id}
            layout={false}
            initial={reduceMotion ? false : { opacity: 0, y: 12, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={reduceMotion ? undefined : { opacity: 0, y: 8, scale: 0.98 }}
            transition={
              reduceMotion
                ? { duration: 0 }
                : { type: "spring", stiffness: 420, damping: 32 }
            }
            className="pointer-events-auto flex items-start gap-2 rounded-md border border-(--_dk-line-visible) bg-(--_dk-overlay) px-3 py-2 text-[13px] shadow-(--_dk-elevation)"
          >
            <span
              className={`mt-[6px] h-1.5 w-1.5 shrink-0 rounded-full ${VARIANT_DOT[toast.variant]}`}
            />
            <p className="min-w-0 flex-1 leading-snug text-(--_dk-text-secondary)">
              {toast.message}
            </p>
            <button
              type="button"
              onClick={() => dismissToast(toast.id)}
              className="shrink-0 rounded px-1 text-[11px] text-(--_dk-text-muted) opacity-70 hover:opacity-100"
              aria-label="Dismiss"
            >
              ×
            </button>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}
