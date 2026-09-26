import { useEffect, useRef, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { useTerminalTabs } from "../../lib/litecodeTerminal";
import { TerminalInstance } from "./terminal/TerminalInstance";

type GroupApi = IDockviewPanelProps["api"]["group"]["api"];

export function TerminalPanel(props: IDockviewPanelProps<{ cwd?: string }>) {
  const tabs = useTerminalTabs((s) => s.tabs);
  const activeKey = useTerminalTabs((s) => s.activeKey);
  const open = useTerminalTabs((s) => s.open);
  const close = useTerminalTabs((s) => s.close);
  const activate = useTerminalTabs((s) => s.activate);
  const collapsed = useCollapsed(props.api.group.api);

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

  return (
    <div className="flex h-full min-h-0 w-full bg-(--_dk-editor)">
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
      <aside className="flex w-40 shrink-0 flex-col border-l border-(--_dk-border-visible)">
        <button
          type="button"
          className="flex h-7 shrink-0 items-center justify-center border-b border-(--_dk-border-visible) text-sm text-(--_dk-text-secondary) hover:bg-(--_dk-surface-raised)"
          aria-label="New Terminal"
          onClick={() => open()}
        >
          +
        </button>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {tabs.map((tab, index) => {
            const isActive = tab.key === activeKey;
            return (
              <div
                key={tab.key}
                className={`group flex h-7 cursor-pointer items-center gap-1 px-2 text-xs text-(--_dk-text-secondary) hover:bg-(--_dk-surface-raised) ${
                  isActive ? "bg-(--_dk-surface-raised)" : ""
                }`}
                onClick={() => activate(tab.key)}
              >
                <span className="w-4 shrink-0 tabular-nums opacity-60">
                  {index + 1}
                </span>
                <span className="min-w-0 flex-1 truncate">{tab.title}</span>
                <button
                  type="button"
                  className="shrink-0 opacity-0 group-hover:opacity-100"
                  aria-label={`Close ${tab.title}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    close(tab.key);
                  }}
                >
                  ×
                </button>
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
