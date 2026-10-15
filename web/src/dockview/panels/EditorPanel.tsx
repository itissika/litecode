// Must stay first: configures @monaco-editor/react to use the local bundle
// before EditorPane (and the Editor) can evaluate or render.
import "../../lib/monaco";
import type { IDockviewPanelProps } from "dockview-react";
import { EditorPane } from "../../components/EditorPane";

export function EditorPanel(props: IDockviewPanelProps<{ filePath: string }>) {
  return <EditorPane filePath={props.params.filePath} api={props.api} />;
}
