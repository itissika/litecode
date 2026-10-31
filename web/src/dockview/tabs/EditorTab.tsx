import { FloppyDiskIcon } from "@phosphor-icons/react";
import type { IDockviewPanelProps } from "dockview-react";

import { useEditorStore } from "../../stores/editorStore";
import { fileNameFromPath } from "../../utils/language";
import { BaseTab, TabIcon, TabTitle } from "./BaseTab";
import { getPanelIcon } from "./icons";

export function EditorTab(props: IDockviewPanelProps<{ filePath: string }>) {
  const filePath = props.params.filePath;
  const tab = useEditorStore((s) =>
    s.tabs.find((t) => t.path === filePath),
  );
  const saving = useEditorStore((s) => s.saving);
  const dirty = tab?.dirty ?? false;
  const canSave = dirty && !tab?.external && tab?.kind === "text";

  const Icon = getPanelIcon(props.api.component);

  return (
    <BaseTab onClose={() => props.api.close()}>
      <TabIcon>
        {dirty ? (
          <span className="text-(--_dk-amber-500) text-xs" aria-label="unsaved">
            ●
          </span>
        ) : (
          <Icon size={14} weight="regular" />
        )}
      </TabIcon>
      <TabTitle>{fileNameFromPath(filePath)}</TabTitle>
      {canSave && (
        <button
          type="button"
          title="Save (Ctrl+S)"
          aria-label="Save file"
          disabled={saving}
          onPointerDown={(event) => {
            // Keep the tab strip from activating or starting a drag before
            // the save click lands.
            event.preventDefault();
            event.stopPropagation();
          }}
          onClick={(event) => {
            event.stopPropagation();
            void useEditorStore.getState().save(filePath);
          }}
          className="rounded p-0.5 opacity-70 hover:opacity-100 hover:bg-(--_dk-ix-bg-hover) transition-colors flex-shrink-0 disabled:opacity-40"
        >
          <FloppyDiskIcon size={12} />
        </button>
      )}
    </BaseTab>
  );
}
