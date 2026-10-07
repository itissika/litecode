import { useEffect, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { getPanelIcon } from "./icons";

export function BrowserTab(props: IDockviewPanelProps) {
  const [title, setTitle] = useState(props.api.title || "Browser");

  useEffect(() => {
    setTitle(props.api.title || "Browser");
    const sub = props.api.onDidTitleChange((event) => {
      setTitle(event.title || "Browser");
    });
    return () => sub.dispose();
  }, [props.api]);

  const Icon = getPanelIcon("browser");

  return (
    <div className="flex items-center gap-1.5 px-1.5 h-full w-full group transition-colors duration-120 hover:brightness-125 active:brightness-75">
      <span className="flex-shrink-0">
        <Icon size={14} weight="regular" />
      </span>
      <span className="text-xs truncate flex-1 min-w-0 select-none">{title}</span>
      <button
        className="rounded p-0.5 opacity-50 hover:opacity-100 hover:bg-(--_dk-ix-danger-bg-hover) hover:text-(--_dk-ix-danger-fg-hover) transition-colors flex-shrink-0"
        title="Close"
        onPointerDown={(event) => {
          event.preventDefault();
          event.stopPropagation();
        }}
        onClick={(event) => {
          event.stopPropagation();
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
