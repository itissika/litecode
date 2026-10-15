import type { ReactNode } from "react";

/** Truncating title span used inside every grid tab. */
export function TabTitle({ children }: { children: ReactNode }) {
  return (
    <span className="text-xs truncate flex-1 min-w-0 select-none">
      {children}
    </span>
  );
}

/** Tab icon slot (fixed-width, never shrinks). */
export function TabIcon({ children }: { children: ReactNode }) {
  return <span className="flex-shrink-0">{children}</span>;
}

/**
 * Shared grid-tab shell: hover/active treatment plus the close button. The
 * caller supplies the leading content (icon / status dot + title).
 */
export function BaseTab({
  onClose,
  children,
}: {
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center gap-1.5 px-1.5 h-full w-full group transition-colors duration-120 hover:brightness-125 active:brightness-75">
      {children}
      <button
        className="rounded p-0.5 opacity-50 hover:opacity-100 hover:bg-(--_dk-ix-danger-bg-hover) hover:text-(--_dk-ix-danger-fg-hover) transition-colors flex-shrink-0"
        title="Close"
        onPointerDown={(e) => {
          // Prevent the tab strip's pointerdown activation from switching to
          // this panel before the click closes it (dockview's default tab
          // does the same). Without this, closing a non-active tab flashes it.
          e.preventDefault();
          e.stopPropagation();
        }}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
      >
        <svg width="10" height="10" viewBox="0 0 15 15" fill="currentColor">
          <path d="M11.78 3.22a.75.75 0 0 1 0 1.06L8.06 8l3.72 3.72a.75.75 0 1 1-1.06 1.06L7 9.06l-3.72 3.72a.75.75 0 0 1-1.06-1.06L5.94 8 2.22 4.28a.75.75 0 0 1 1.06-1.06L7 6.94l3.72-3.72a.75.75 0 0 1 1.06 0Z" />
        </svg>
      </button>
    </div>
  );
}
