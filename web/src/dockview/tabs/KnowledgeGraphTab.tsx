import type { IDockviewPanelProps } from "dockview-react";

import { getPanelIcon } from "./icons";

/** Grid tab for the singleton knowledge-graph panel: icon + title + close. */
export function KnowledgeGraphTab(props: IDockviewPanelProps) {
  const title = props.api.title ?? props.api.id;
  const Icon = getPanelIcon(props.api.component);

  return (
    <div className="flex items-center gap-1.5 px-1.5 h-full w-full transition-colors duration-120 hover:brightness-125 active:brightness-75">
      <span className="flex-shrink-0">
        <Icon size={14} weight="regular" />
      </span>
      <span className="text-xs truncate flex-1 min-w-0 select-none">
        {title}
      </span>
      <button
        className="rounded p-0.5 opacity-50 hover:opacity-100 hover:bg-(--_dk-ix-danger-bg-hover) hover:text-(--_dk-ix-danger-fg-hover) transition-colors flex-shrink-0"
        title="Close"
        onPointerDown={(e) => {
          // Prevent the tab strip's pointerdown activation from switching to
          // this panel before the click closes it (same as the default tab).
          e.preventDefault();
          e.stopPropagation();
        }}
        onClick={(e) => {
          e.stopPropagation();
          props.api.close();
        }}
      >
        <svg width="10" height="10" viewBox="0 0 15 15" fill="currentColor">
          <path d="M11.78 3.22a.75.75 0 0 1 0 1.06L8.06 8l3.72 3.72a.75.75 0 1 1-1.06 1.06L7 9.06l-3.72 3.72a.75.75 0 0 1-1.06-1.06L5.94 8 2.22 4.28a.75.75 0 0 1 1.06-1.06L7 6.94l3.72-3.72a.75.75 0 0 1 1.06 0Z" />
        </svg>
      </button>
    </div>
  );
}
