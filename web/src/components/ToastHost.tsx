import { Check, Copy } from "@phosphor-icons/react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useState } from "react";

import {
  useToastStore,
  type Toast,
  type ToastVariant,
} from "../stores/toastStore";
import { ToastMarkdown } from "./ToastMarkdown";

const VARIANT_DOT: Record<ToastVariant, string> = {
  success: "bg-(--_dk-emerald-500)",
  error: "bg-(--_dk-red-500)",
  info: "bg-(--_dk-text-muted)",
};

/** Copy the raw note text — an error is meant to be pasted somewhere else. */
function CopyRawButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const label = copied ? "Copied" : "Copy error message";

  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard
          .writeText(text)
          .then(() => setCopied(true))
          .catch(() => {});
      }}
      className="absolute top-0.5 right-0.5 rounded p-1 text-(--_dk-text-muted) opacity-70 hover:opacity-100"
      aria-label={label}
      title={label}
    >
      {copied ? (
        <Check size={12} weight="bold" aria-hidden />
      ) : (
        <Copy size={12} aria-hidden />
      )}
    </button>
  );
}

function ToastRow({
  toast,
  reduceMotion,
  onDismiss,
}: {
  toast: Toast;
  reduceMotion: boolean | null;
  onDismiss: () => void;
}) {
  return (
    <motion.div
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
      {toast.icon ? (
        <span className="mt-[1px] shrink-0 text-[13px] leading-[18px]">
          {toast.icon}
        </span>
      ) : (
        <span
          className={`mt-[6px] h-1.5 w-1.5 shrink-0 rounded-full ${VARIANT_DOT[toast.variant]}`}
        />
      )}
      <div className="min-w-0 flex-1 text-(--_dk-text-secondary)">
        {toast.variant === "error" ? (
          // Errors stay raw — markdown would eat their punctuation — inside
          // their own scrolling list, one click away from being copied.
          <div className="relative rounded-sm border border-(--_dk-line)">
            <div
              className="max-h-[min(50vh,14rem)] overflow-y-auto overscroll-contain py-1.5 pr-7 pl-2 font-mono text-[12px] leading-[1.5] whitespace-pre-wrap break-words"
              role="group"
              aria-label="Error details"
            >
              {toast.message}
            </div>
            <CopyRawButton text={toast.message} />
          </div>
        ) : (
          <div className="max-h-[min(50vh,14rem)] overflow-y-auto overscroll-contain">
            <ToastMarkdown text={toast.message} />
          </div>
        )}
      </div>
      {toast.count > 1 ? (
        <span
          className="mt-0.5 shrink-0 rounded-full bg-(--_dk-side) px-1.5 text-[11px] leading-4 text-(--_dk-text-muted)"
          title={`${toast.count} identical notes`}
          aria-label={`${toast.count} identical notes`}
        >
          {toast.count}
        </span>
      ) : null}
      <button
        type="button"
        onClick={onDismiss}
        className="shrink-0 rounded px-1 text-[11px] text-(--_dk-text-muted) opacity-70 hover:opacity-100"
        aria-label="Dismiss"
      >
        ×
      </button>
    </motion.div>
  );
}

export function ToastHost() {
  const toasts = useToastStore((s) => s.toasts);
  const dismissToast = useToastStore((s) => s.dismissToast);
  const setPaused = useToastStore((s) => s.setPaused);
  const reduceMotion = useReducedMotion();

  // Unmounting must never strand a held timer.
  useEffect(() => () => setPaused(false), [setPaused]);

  return (
    <div
      className="pointer-events-none fixed bottom-4 right-4 z-100 flex max-w-sm flex-col gap-2"
      aria-live="polite"
      // Hovering any note holds every note: reading time is never stolen.
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <AnimatePresence initial={false}>
        {toasts.map((toast) => (
          <ToastRow
            key={toast.id}
            toast={toast}
            reduceMotion={reduceMotion}
            onDismiss={() => dismissToast(toast.id)}
          />
        ))}
      </AnimatePresence>
    </div>
  );
}
