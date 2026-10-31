import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { IDockviewPanelProps } from "dockview-react";

import { useEditorStore } from "../../stores/editorStore";
import { EditorTab } from "./EditorTab";

const PATH = "src/a.ts";

function panelProps(): IDockviewPanelProps<{ filePath: string }> {
  return {
    params: { filePath: PATH },
    api: { component: "editor", close: vi.fn() },
  } as unknown as IDockviewPanelProps<{ filePath: string }>;
}

afterEach(() => {
  cleanup();
  useEditorStore.setState({ tabs: [], saving: false });
});

describe("EditorTab save button", () => {
  it("saves a dirty tab by path", () => {
    const save = vi.fn(async () => {});
    useEditorStore.setState({
      save,
      tabs: [
        {
          path: PATH,
          content: "edited",
          savedContent: "",
          dirty: true,
          language: "typescript",
          loading: false,
          error: null,
          errorRetryable: false,
          kind: "text",
          diskRevision: 0,
        },
      ],
    });
    render(<EditorTab {...panelProps()} />);

    fireEvent.click(screen.getByRole("button", { name: "Save file" }));

    expect(save).toHaveBeenCalledWith(PATH);
  });

  it("stays out of the way for a clean tab", () => {
    useEditorStore.setState({
      tabs: [
        {
          path: PATH,
          content: "same",
          savedContent: "same",
          dirty: false,
          language: "typescript",
          loading: false,
          error: null,
          errorRetryable: false,
          kind: "text",
          diskRevision: 0,
        },
      ],
    });
    render(<EditorTab {...panelProps()} />);

    expect(screen.queryByRole("button", { name: "Save file" })).toBeNull();
  });
});
