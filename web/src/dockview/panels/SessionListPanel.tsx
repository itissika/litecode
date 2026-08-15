import type { IDockviewPanelProps } from "dockview-react";
import { SessionList } from "../../components/SessionList";

export function SessionListPanel(_props: IDockviewPanelProps) {
  return (
    <div className="flex h-full flex-col">
      <SessionList />
    </div>
  );
}
