import { createPortal } from "react-dom";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

export interface FileTreeMenuItem {
  id: string;
  label?: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
  onClick?: () => void;
}

interface FileTreeContextMenuProps {
  x: number;
  y: number;
  items: FileTreeMenuItem[];
  onClose: () => void;
}

export function FileTreeContextMenu({ x, y, items, onClose }: FileTreeContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const left = Math.min(x, window.innerWidth - rect.width - 8);
    const top = Math.min(y, window.innerHeight - rect.height - 8);
    setPos({ left: Math.max(8, left), top: Math.max(8, top) });
  }, [x, y, items.length]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={ref}
      role="menu"
      className="fixed z-50 min-w-[198px] rounded border border-(--_dk-line) bg-(--_dk-editor) py-1 text-sm shadow-lg"
      style={{ left: pos.left, top: pos.top }}
    >
      {items.map((item) =>
        item.separator ? (
          <div
            key={item.id}
            className="my-1 border-t border-(--_dk-line)"
            role="separator"
          />
        ) : (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            className={`flex w-full items-center justify-between gap-4 px-3 py-1 text-left disabled:opacity-40 ${
              item.danger
                ? "text-(--_dk-ix-danger-fg) hover:bg-(--_dk-ix-danger-bg-hover)"
                : "text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover)"
            }`}
            onClick={() => {
              if (item.disabled) return;
              item.onClick?.();
              onClose();
            }}
          >
            <span>{item.label}</span>
            {item.shortcut && (
              <span className="text-[10px] text-(--_dk-text-disabled)">{item.shortcut}</span>
            )}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}
