import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { TerminalWindow, Trash } from "@phosphor-icons/react";

import { useTerminalTabs } from "../../lib/litecodeTerminal";
import { TerminalInstance } from "./terminal/TerminalInstance";

type GroupApi = IDockviewPanelProps["api"]["group"]["api"];

/** Sidebar geometry: the dockview sash is a 4px hit strip, so the drag handle
 *  between the terminal and the list is the same width. */
const SIDEBAR_WIDTH_KEY = "litecode-terminal-sidebar-width";
const SIDEBAR_MIN_W = 120;
/** Fallback ceiling while the pane has not been measured (jsdom reports 0). */
const SIDEBAR_MAX_W = 420;
/** Share of the pane the terminal itself keeps. */
const TERMINAL_MIN_W = 200;

function clampWidth(width: number, paneWidth: number): number {
  const max =
    paneWidth > 0
      ? Math.min(SIDEBAR_MAX_W, Math.max(SIDEBAR_MIN_W, paneWidth - TERMINAL_MIN_W))
      : SIDEBAR_MAX_W;
  return Math.min(max, Math.max(SIDEBAR_MIN_W, Math.round(width)));
}

function readSidebarWidth(): number {
  const raw = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
  if (!Number.isFinite(raw) || raw <= 0) return 160;
  return clampWidth(raw, 0);
}

export function TerminalPanel(props: IDockviewPanelProps<{ cwd?: string }>) {
  const tabs = useTerminalTabs((s) => s.tabs);
  const activeKey = useTerminalTabs((s) => s.activeKey);
  const open = useTerminalTabs((s) => s.open);
  const close = useTerminalTabs((s) => s.close);
  const activate = useTerminalTabs((s) => s.activate);
  const collapsed = useCollapsed(props.api.group.api);

  const rootRef = useRef<HTMLDivElement>(null);
  const [sidebarWidth, setSidebarWidth] = useState(readSidebarWidth);
  const sidebarWidthRef = useRef(sidebarWidth);
  sidebarWidthRef.current = sidebarWidth;
  const [dragging, setDragging] = useState(false);
  const draggingRef = useRef(false);
  const dragStartRef = useRef({ x: 0, width: sidebarWidth });

  // First paint while the edge is open, and every later expand, starts a
  // shell when the list is empty. Closing the last tab while the edge is
  // open leaves the empty state until the next expand.
  const didInit = useRef(false);
  const wasCollapsed = useRef(collapsed);
  useEffect(() => {
    if (!didInit.current) {
      didInit.current = true;
      wasCollapsed.current = collapsed;
      if (!collapsed && useTerminalTabs.getState().tabs.length === 0) {
        open(props.params?.cwd);
      }
      return;
    }
    if (wasCollapsed.current && !collapsed) {
      if (useTerminalTabs.getState().tabs.length === 0) open();
    }
    wasCollapsed.current = collapsed;
  }, [collapsed, open, props.params?.cwd]);

  // Drag the seam. Same pointer-capture pattern as the dockview sash (and
  // AgentChatInput): the handle owns the gesture, `userSelect: none` keeps the
  // drag off the text, and the width is written back only when it settles.
  const onResizeStart = (e: ReactPointerEvent) => {
    e.preventDefault();
    draggingRef.current = true;
    setDragging(true);
    dragStartRef.current = { x: e.clientX, width: sidebarWidthRef.current };
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    document.body.style.userSelect = "none";
    document.body.style.cursor = "ew-resize";
  };

  const onResizeMove = (e: ReactPointerEvent) => {
    if (!draggingRef.current) return;
    // Dragging left widens the sidebar.
    const paneWidth = rootRef.current?.offsetWidth ?? 0;
    const pulled = e.clientX - dragStartRef.current.x;
    setSidebarWidth(clampWidth(dragStartRef.current.width - pulled, paneWidth));
  };

  const onResizeEnd = (e: ReactPointerEvent) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    setDragging(false);
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture?.(e.pointerId)) {
      el.releasePointerCapture?.(e.pointerId);
    }
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
    localStorage.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidthRef.current));
  };

  return (
    <div ref={rootRef} className="flex h-full min-h-0 w-full bg-(--_dk-editor)">
      <div className="relative min-h-0 min-w-0 flex-1">
        {tabs.map((tab) => {
          const isActive = tab.key === activeKey;
          return (
            <div
              key={tab.key}
              className="absolute inset-1"
              style={{ visibility: isActive ? "visible" : "hidden" }}
            >
              <TerminalInstance
                tabKey={tab.key}
                cwd={tab.cwd}
                active={isActive}
                expanded={!collapsed}
                onExited={() => close(tab.key)}
              />
            </div>
          );
        })}
        {tabs.length === 0 && (
          <div className="flex h-full items-center justify-center">
            <button
              type="button"
              className="rounded border border-(--_dk-border-visible) px-2 py-1 text-xs text-(--_dk-text-secondary) hover:bg-(--_dk-surface-raised)"
              onClick={() => open()}
            >
              New Terminal
            </button>
          </div>
        )}
      </div>
      {/* Resize seam. The 4px strip is pure hit area: a 1px hairline sits in
          its middle so the divider is visible at rest, and the strip washes in
          on hover / while dragging — the dockview sash's own feedback, minus
          the invisible-at-rest state. */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize terminal list"
        data-testid="terminal-sidebar-resize"
        onPointerDown={onResizeStart}
        onPointerMove={onResizeMove}
        onPointerUp={onResizeEnd}
        onPointerCancel={onResizeEnd}
        className={`relative w-1 shrink-0 cursor-ew-resize transition-colors duration-120 after:pointer-events-none after:absolute after:inset-y-0 after:left-1/2 after:w-px after:-translate-x-1/2 after:bg-(--_dk-border-visible) after:content-[''] ${
          dragging
            ? "bg-(--_dk-line-visible)"
            : "bg-transparent hover:bg-(--_dk-ix-bg-hover)"
        }`}
      />
      <aside
        data-testid="terminal-sidebar"
        className="flex shrink-0 flex-col"
        style={{ width: sidebarWidth }}
      >
        <div className="relative flex h-7 shrink-0 items-center justify-end px-1.5">
          <button
            type="button"
            className="flex h-5 w-5 items-center justify-center rounded text-sm leading-none text-(--_dk-text-muted) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary)"
            aria-label="New Terminal"
            title="New Terminal"
            onClick={() => open()}
          >
            +
          </button>
          {/* The header IS its divider: a hairline running the full width, no
              side insets. */}
          <span
            aria-hidden
            data-testid="terminal-header-divider"
            className="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-(--_dk-line-visible)"
          />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {tabs.map((tab) => {
            const label = tab.lastCommand || tab.shell || tab.title;
            const isActive = tab.key === activeKey;
            return (
              <div
                key={tab.key}
                className={`group flex w-full cursor-pointer items-center gap-1 truncate px-2 py-0.5 text-left text-sm transition-colors hover:bg-(--_dk-ix-bg-hover) ${
                  isActive
                    ? "text-(--_dk-text-primary)"
                    : "text-(--_dk-text-secondary)"
                }`}
                title={tab.cwd ?? tab.title}
                onClick={() => activate(tab.key)}
              >
                <TerminalWindow
                  size={16}
                  weight="regular"
                  aria-hidden
                  className="h-4 w-4 shrink-0 select-none"
                />
                <span className="truncate">{label}</span>
                <span className="ml-auto flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    className="pointer-events-none rounded text-(--_dk-text-muted) opacity-0 transition-opacity group-hover:pointer-events-auto group-hover:opacity-100 hover:text-(--_dk-ix-danger-fg-hover) focus-visible:opacity-100"
                    aria-label={`Close ${tab.title}`}
                    title="Close terminal"
                    onClick={(e) => {
                      e.stopPropagation();
                      close(tab.key);
                    }}
                  >
                    <Trash size={14} />
                  </button>
                </span>
              </div>
            );
          })}
        </div>
      </aside>
    </div>
  );
}

function useCollapsed(groupApi: GroupApi): boolean {
  const [collapsed, setCollapsed] = useState(() => groupApi.isCollapsed());
  useEffect(() => {
    setCollapsed(groupApi.isCollapsed());
    const sub = groupApi.onDidCollapsedChange((event) => {
      setCollapsed(event.isCollapsed);
    });
    return () => sub.dispose();
  }, [groupApi]);
  return collapsed;
}
