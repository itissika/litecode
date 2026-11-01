import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { InlineRename } from "./InlineRename";

function renderEditor(overrides: Partial<Parameters<typeof InlineRename>[0]> = {}) {
  const onCommit = vi.fn();
  const onCancel = vi.fn();
  const onInvalid = vi.fn();
  render(
    <InlineRename
      initialValue="cover.png"
      ariaLabel="Rename"
      onCommit={onCommit}
      onCancel={onCancel}
      onInvalid={onInvalid}
      {...overrides}
    />,
  );
  return {
    input: screen.getByRole("textbox", { name: "Rename" }) as HTMLInputElement,
    onCommit,
    onCancel,
    onInvalid,
  };
}

afterEach(cleanup);

describe("InlineRename", () => {
  it("selects the whole value by default", () => {
    const { input } = renderEditor();
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 9]);
  });

  it("can select only the filename stem", () => {
    const { input } = renderEditor({ selection: "stem" });
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 5]);
  });

  it("commits on Enter with the trimmed value", () => {
    const { input, onCommit } = renderEditor();
    fireEvent.change(input, { target: { value: "  new name  " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith("new name");
  });

  it("cancels an empty draft by default", () => {
    const { input, onCancel, onCommit } = renderEditor();
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("commits an empty draft when allowEmpty is set", () => {
    const { input, onCommit } = renderEditor({ allowEmpty: true });
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith("");
  });

  it("keeps the editor open and reports invalid values", () => {
    const { input, onCommit, onInvalid } = renderEditor({
      validate: () => "bad-name",
      allowEmpty: true,
    });
    fireEvent.change(input, { target: { value: "bad/name" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onInvalid).toHaveBeenCalledWith("bad-name");
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("does not cancel twice on Escape followed by blur", () => {
    const { input, onCancel } = renderEditor();
    fireEvent.keyDown(input, { key: "Escape" });
    fireEvent.blur(input);
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("forwards placeholder and spellcheck", () => {
    const { input } = renderEditor({
      initialValue: "",
      placeholder: "Folder name",
      spellCheck: false,
    });
    expect(input.placeholder).toBe("Folder name");
    expect(input.getAttribute("spellcheck")).toBe("false");
  });
});
