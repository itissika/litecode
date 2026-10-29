import type { IDockviewPanelProps } from "dockview-react";

import { BaseTab, TabIcon, TabTitle } from "./BaseTab";
import { getPanelIcon } from "./icons";
import { AppIcon } from "../../components/apps/AppIcon";
import { useAppsStore } from "../../stores/appsStore";

export function AppTab(
  props: IDockviewPanelProps<{ appId?: string }>,
) {
  const appId =
    typeof props.params?.appId === "string"
      ? props.params.appId
      : props.api.id.startsWith("app-")
        ? props.api.id.slice(4)
        : "";
  const app = useAppsStore((state) =>
    state.apps.find((item) => item.id === appId),
  );
  const title = props.api.title || app?.name || props.api.id;
  const Icon = getPanelIcon("appview");
  return (
    <BaseTab onClose={() => props.api.close()}>
      <TabIcon>
        {app ? <AppIcon app={app} size={14} /> : <Icon size={14} />}
      </TabIcon>
      <TabTitle>{title}</TabTitle>
    </BaseTab>
  );
}
