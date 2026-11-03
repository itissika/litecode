import { createPortal } from "react-dom";
import { useId, useLayoutEffect, useRef, type KeyboardEvent } from "react";

import { usePortalTarget } from "./usePortalTarget";

export interface ConfirmDialogProps {
  title: string;
  body?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * App-native destructive confirmation. The hook owns the promise; this owns
 * focus, Escape/overlay dismissal, and the portal into the panel's document.
 */
export function ConfirmDialog({
  title,
  body,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  danger = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const titleId = useId();
  const bodyId = useId();
  const { anchorRef, target } = usePortalTarget<HTMLSpanElement>();
  const panelRef = useRef<HTMLDivElement>(null);
  const activeElementRef = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    const active = anchor?.ownerDocument.activeElement;
    activeElementRef.current = active instanceof HTMLElement ? active : null;
  }, [anchorRef]);

  useLayoutEffect(() => {
    if (!target) return;
    panelRef.current
      ?.querySelector<HTMLElement>("[data-confirm-cancel]")
      ?.focus();
  }, [target]);

  useLayoutEffect(() => {
    return () => {
      const active = activeElementRef.current;
      if (active?.isConnected) active.focus();
    };
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
      return;
    }
    if (event.key !== "Tab") return;
    const panel = panelRef.current;
    if (!panel) return;
    const focusable = [...panel.querySelectorAll<HTMLElement>("button:not([disabled])")];
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = panel.ownerDocument.activeElement;
    if (event.shiftKey && active === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <>
      <span ref={anchorRef} hidden aria-hidden />
      {target
        ? createPortal(
            <div
              data-litecode-overlay="confirm-dialog"
              className="fixed inset-0 z-(--_dk-z-dialog) flex items-center justify-center bg-(--_dk-root)/60 p-4"
              onMouseDown={(event) => {
                if (event.target === event.currentTarget) onCancel();
              }}
            >
              <div
                ref={panelRef}
                role="dialog"
                aria-modal="true"
                aria-labelledby={titleId}
                aria-describedby={body ? bodyId : undefined}
                onKeyDown={onKeyDown}
                className="w-full max-w-sm rounded-lg border border-(--_dk-line-visible) bg-(--_dk-overlay) p-4 text-(--_dk-text-secondary) shadow-(--_dk-elevation)"
              >
                <h2 id={titleId} className="text-sm font-medium text-(--_dk-text-primary)">
                  {title}
                </h2>
                {body ? (
                  <p
                    id={bodyId}
                    className="mt-2 whitespace-pre-wrap break-words text-sm"
                  >
                    {body}
                  </p>
                ) : null}
                <div className="mt-4 flex justify-end gap-2">
                  <button
                    type="button"
                    data-confirm-cancel
                    className="btn btn-sm btn-ghost"
                    onClick={onCancel}
                  >
                    {cancelLabel}
                  </button>
                  <button
                    type="button"
                    className={`btn btn-sm ${danger ? "btn-danger" : "btn-primary"}`}
                    onClick={onConfirm}
                  >
                    {confirmLabel}
                  </button>
                </div>
              </div>
            </div>,
            target,
          )
        : null}
    </>
  );
}
