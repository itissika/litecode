import { lazy, Suspense } from "react";
import type { IDockviewPanelProps } from "dockview-react";

// Monaco (and its local-bundle loader config) loads with the first editor
// panel, not at boot. EditorPanel imports lib/monaco before EditorPane.
const EditorPanel = lazy(() =>
  import("../panels/EditorPanel").then((m) => ({ default: m.EditorPanel })),
);

export function LazyEditorPanel(
  props: IDockviewPanelProps<{ filePath: string }>,
) {
  return (
    <Suspense fallback={<div className="h-full w-full bg-(--_dk-editor)" />}>
      <EditorPanel {...props} />
    </Suspense>
  );
}
