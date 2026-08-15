import type { IDockviewPanelProps } from "dockview-react";
import { EditorPane } from "../../components/EditorPane";

export function EditorPanel(props: IDockviewPanelProps<{ filePath: string }>) {
  return <EditorPane filePath={props.params.filePath} api={props.api} />;
}
