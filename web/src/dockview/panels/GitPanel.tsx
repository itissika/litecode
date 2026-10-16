import { useEffect } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { GitPanel } from "../../components/GitPanel";
import { useGitStore } from "../../stores/gitStore";

export function GitPanelHost(props: IDockviewPanelProps) {
  const api = props.api;

  useEffect(() => {
    // Status keeps updating while this panel is hidden (file badges share it).
    // Showing it again pulls the commit log.
    useGitStore.getState().setVisible(api.isVisible);
    const sub = api.onDidVisibilityChange((e) => {
      useGitStore.getState().setVisible(e.isVisible);
    });
    return () => {
      sub.dispose();
      useGitStore.getState().setVisible(false);
    };
  }, [api]);

  return <GitPanel />;
}
