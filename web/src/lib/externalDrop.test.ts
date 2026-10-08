import { afterEach, describe, expect, it, vi } from "vitest";

import { useEditorStore } from "../stores/editorStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { foreignDropIntent, previewForeignDrop } from "./externalDrop";
import { fakeTransfer } from "../test/fakeTransfer";

afterEach(() => {
  document.body.replaceChildren();
  useEditorStore.setState({
    tabs: [],
    activePath: null,
    conflicts: {},
    saving: false,
  });
  useSessionStore.setState({ project: "" });
  Reflect.deleteProperty(window, "litecode");
});

function zone(name: string): HTMLElement {
  const root = document.createElement("div");
  root.setAttribute(
    name === "mention" ? "data-mention-drop" : "data-drop-zone",
    name === "mention" ? "true" : name,
  );
  const child = document.createElement("span");
  root.append(child);
  document.body.append(root);
  return child;
}

describe("foreign drop routing", () => {
  it("previews editor drops, swallows terminal drops, and leaves the tree and chat alone", () => {
    const file = new File(["a"], "a.ts", { type: "text/plain" });
    const dt = fakeTransfer({ files: [file] });
    expect(foreignDropIntent(zone("editor"), dt, "drop")).toBe("preview");
    expect(foreignDropIntent(zone("terminal"), dt, "over")).toBe("swallow");
    expect(foreignDropIntent(zone("tree"), dt, "drop")).toBe("ignore");
    expect(foreignDropIntent(zone("mention"), dt, "drop")).toBe("ignore");
    expect(foreignDropIntent(document.createElement("div"), dt, "drop")).toBe("ignore");
  });

  it("opens a temporary preview for a dropped file and a workspace file for a path", async () => {
    const file = new File(["hello"], "notes.ts", { type: "text/plain" });
    const files = fakeTransfer({ files: [file] });
    window.litecode = { getPathForFile: () => "C:\\outside\\notes.ts" };
    await previewForeignDrop(files);
    const tab = useEditorStore.getState().tabs[0];
    expect(tab?.external).toBe(true);
    expect(tab?.content).toBe("hello");
    expect(tab?.path).toBe("external:C:/outside/notes.ts");

    useSessionStore.setState({ project: "E:/ws" });
    const openFile = vi.fn(async () => {});
    const previous = useEditorStore.getState().openFile;
    useEditorStore.setState({ openFile });
    const uri = fakeTransfer();
    uri.setData("text/uri-list", "file:///E:/ws/src/a.ts");
    await previewForeignDrop(uri);
    expect(openFile).toHaveBeenCalledWith("src/a.ts");
    useEditorStore.setState({ openFile: previous });
  });

  it("does not open a folder placeholder", async () => {
    const showToast = vi.spyOn(useToastStore.getState(), "showToast");
    const folder = new File([], "dir");
    const dt = fakeTransfer({ files: [folder] });
    await previewForeignDrop(dt);
    expect(useEditorStore.getState().tabs).toEqual([]);
    expect(showToast).toHaveBeenCalled();
    showToast.mockRestore();
  });
});
