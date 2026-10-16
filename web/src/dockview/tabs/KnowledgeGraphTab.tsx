import type { IDockviewPanelProps } from "dockview-react";

import { BaseTab, TabIcon, TabTitle } from "./BaseTab";
import { getPanelIcon } from "./icons";

/** Grid tab for the singleton knowledge-graph panel: icon + title + close. */
export function KnowledgeGraphTab(props: IDockviewPanelProps) {
  const title = props.api.title ?? props.api.id;
  const Icon = getPanelIcon(props.api.component);

  return (
    <BaseTab onClose={() => props.api.close()}>
      <TabIcon>
        <Icon size={14} weight="regular" />
      </TabIcon>
      <TabTitle>{title}</TabTitle>
    </BaseTab>
  );
}
