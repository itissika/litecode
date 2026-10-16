import { useEffect, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { BaseTab, TabIcon, TabTitle } from "./BaseTab";
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
    <BaseTab onClose={() => props.api.close()}>
      <TabIcon>
        <Icon size={14} weight="regular" />
      </TabIcon>
      <TabTitle>{title}</TabTitle>
    </BaseTab>
  );
}
