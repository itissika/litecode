import type { IDockviewPanelProps } from "dockview-react";

import { useEditorStore } from "../../stores/editorStore";
import { fileNameFromPath } from "../../utils/language";
import { getPanelIcon } from "./icons";

export function EditorTab(props: IDockviewPanelProps<{ filePath: string }>) {
  const filePath = props.params.filePath;
  const dirty = useEditorStore((s) => {
    const tab = s.tabs.find((t) => t.path === filePath);
    return tab?.dirty ?? false;
  });

  const Icon = getPanelIcon(props.api.component);

  return (
    <div className="flex items-center gap-1.5 px-1.5 h-full w-full group transition-colors duration-120 hover:brightness-125 active:brightness-75">
      <span className="flex-shrink-0">
        {dirty ? (
          <span className="text-(--_dk-amber-500) text-xs" aria-label="unsaved">●</span>
        ) : (
          <Icon size={14} weight="regular" />
        )}
      </span>
      <span className="text-xs truncate flex-1 min-w-0 select-none">
        {fileNameFromPath(filePath)}
      </span>
      <button
        className="rounded p-0.5 opacity-50 hover:opacity-100 hover:text-(--_dk-red-500) transition-opacity flex-shrink-0"
        title="Close"
        onClick={(e) => { e.stopPropagation(); props.api.close(); }}
      >
        <svg width="10" height="10" viewBox="0 0 15 15" fill="currentColor">
          <path d="M11.78 3.22a.75.75 0 0 1 0 1.06L8.06 8l3.72 3.72a.75.75 0 1 1-1.06 1.06L7 9.06l-3.72 3.72a.75.75 0 0 1-1.06-1.06L5.94 8 2.22 4.28a.75.75 0 0 1 1.06-1.06L7 6.94l3.72-3.72a.75.75 0 0 1 1.06 0Z" />
        </svg>
      </button>
    </div>
  );
}
