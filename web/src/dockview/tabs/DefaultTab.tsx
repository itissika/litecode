import type { IDockviewPanelProps } from "dockview-react";

import { BaseTab, TabIcon, TabTitle } from "./BaseTab";
import { getPanelIcon } from "./icons";

/**
 * Grid tab for panels that do not have a specialized tab. Panels opt into it
 * with `tabComponent: "default"`; it must be registered because DockviewReact
 * resolves the name from `tabComponents` and throws on an unregistered one.
 */
export function DefaultTab(props: IDockviewPanelProps) {
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
