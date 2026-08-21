import { useEffect } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { GitPanel } from "../../components/GitPanel";
import { useGitStore } from "../../stores/gitStore";

export function GitPanelHost(props: IDockviewPanelProps) {
  const api = props.api;

  useEffect(() => {
    // Report panel visibility so the store skips refreshes while hidden (the
    // panel stays mounted in dockview, but its data is not being viewed).
    // Becoming visible triggers a catch-up refresh.
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
