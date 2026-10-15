import type { IDockviewPanelProps } from "dockview-react";

import { useEditorStore } from "../../stores/editorStore";
import { fileNameFromPath } from "../../utils/language";
import { BaseTab, TabIcon, TabTitle } from "./BaseTab";
import { getPanelIcon } from "./icons";

export function EditorTab(props: IDockviewPanelProps<{ filePath: string }>) {
  const filePath = props.params.filePath;
  const dirty = useEditorStore((s) => {
    const tab = s.tabs.find((t) => t.path === filePath);
    return tab?.dirty ?? false;
  });

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
    </BaseTab>
  );
}
