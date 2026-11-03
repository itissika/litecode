import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";

import { useConfirmDialog } from "./useConfirmDialog";

afterEach(cleanup);

function Harness() {
  const { confirm, dialog } = useConfirmDialog();
  const [result, setResult] = useState("idle");
  return (
    <>
      <button
        type="button"
        onClick={() => {
          void confirm({
            title: "Delete item?",
            body: "This cannot be undone.",
            danger: true,
            confirmLabel: "Delete",
            cancelLabel: "Cancel",
          }).then((ok) => setResult(ok ? "confirmed" : "cancelled"));
        }}
      >
        Ask
      </button>
      <output>{result}</output>
      {dialog}
    </>
  );
}

function ask() {
  render(<Harness />);
  const trigger = screen.getByRole("button", { name: "Ask" });
  trigger.focus();
  fireEvent.click(trigger);
  return trigger;
}

describe("useConfirmDialog", () => {
  it("focuses cancel and resolves true on confirm", async () => {
    const trigger = ask();
    await screen.findByRole("dialog");
    const cancel = screen.getByRole("button", { name: "Cancel" });
    await waitFor(() => expect(document.activeElement).toBe(cancel));

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("confirmed")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("resolves false on cancel", async () => {
    ask();
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(await screen.findByText("cancelled")).toBeTruthy();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("resolves false on Escape", async () => {
    ask();
    const dialog = await screen.findByRole("dialog");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(await screen.findByText("cancelled")).toBeTruthy();
  });

  it("resolves false on overlay mousedown", async () => {
    ask();
    const dialog = await screen.findByRole("dialog");
    fireEvent.mouseDown(dialog.parentElement as HTMLElement);
    expect(await screen.findByText("cancelled")).toBeTruthy();
  });

  it("renders the body as text", async () => {
    ask();
    expect(await screen.findByText("This cannot be undone.")).toBeTruthy();
  });
});
