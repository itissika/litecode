import { revealEdgePanel } from "./edges";
import { subscribeWindows } from "./windows";
import { useEditorStore } from "../../stores/editorStore";

/**
 * Workbench shortcuts, one listener per registered window.
 * The handler closes over the main window's stores: a key pressed in a popout
 * still saves and reveals search through the same panel manager.
 */
export function bindWorkbenchKeys(): () => void {
  return subscribeWindows((entry) => {
    const onKey = (event: KeyboardEvent) => {
      if (
        (event.ctrlKey || event.metaKey) &&
        event.shiftKey &&
        (event.key === "f" || event.key === "F")
      ) {
        event.preventDefault();
        revealEdgePanel("search");
        window.dispatchEvent(new Event("litecode:focus-workspace-search"));
        return;
      }
      if ((event.ctrlKey || event.metaKey) && (event.key === "s" || event.key === "S")) {
        event.preventDefault();
        void useEditorStore.getState().save();
      }
    };
    // Capture before the focused widget. Monaco's keybinding service sits on
    // the editor container and consumes the keys it maps; a workbench command
    // such as save must not depend on the editor letting the event bubble.
    entry.window.addEventListener("keydown", onKey, true);
    return () => entry.window.removeEventListener("keydown", onKey, true);
  });
}
