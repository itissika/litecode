import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ContextMenu, type ContextMenuItem } from "./ContextMenu";

afterEach(cleanup);

function Harness({ items }: { items: ContextMenuItem[] }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Trigger
      </button>
      {open ? (
        <ContextMenu
          x={10}
          y={20}
          items={items}
          onClose={() => setOpen(false)}
          ariaLabel="Test menu"
        />
      ) : null}
    </>
  );
}

const items = (onAction: (id: string) => void = () => {}): ContextMenuItem[] => [
  { id: "rename", label: "Rename", onClick: () => onAction("rename") },
  { id: "disabled", label: "Disabled", disabled: true },
  { id: "sep", separator: true },
  { id: "delete", label: "Delete", danger: true, onClick: () => onAction("delete") },
];

function openMenu() {
  render(<Harness items={items()} />);
  const trigger = screen.getByRole("button", { name: "Trigger" });
  trigger.focus();
  fireEvent.click(trigger);
  return trigger;
}

describe("ContextMenu", () => {
  it("focuses the first enabled item and roves with arrows", async () => {
    openMenu();
    const rename = await screen.findByRole("menuitem", { name: "Rename" });
    await waitFor(() => expect(document.activeElement).toBe(rename));

    fireEvent.keyDown(rename, { key: "ArrowDown" });
    expect(document.activeElement).toBe(
      screen.getByRole("menuitem", { name: "Delete" }),
    );

    fireEvent.keyDown(screen.getByRole("menuitem", { name: "Delete" }), {
      key: "ArrowUp",
    });
    expect(document.activeElement).toBe(rename);
  });

  it("runs an item, closes, and returns focus to the trigger", async () => {
    const onAction = vi.fn();
    render(<Harness items={items(onAction)} />);
    const trigger = screen.getByRole("button", { name: "Trigger" });
    trigger.focus();
    fireEvent.click(trigger);

    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename" }));
    expect(onAction).toHaveBeenCalledWith("rename");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("closes on Escape with focus restored", async () => {
    const trigger = openMenu();
    await screen.findByRole("menuitem", { name: "Rename" });
    fireEvent.keyDown(screen.getByRole("menuitem", { name: "Rename" }), {
      key: "Escape",
    });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("closes on outside mousedown without stealing focus back", async () => {
    openMenu();
    await screen.findByRole("menuitem", { name: "Rename" });
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("renders separators and disabled items", async () => {
    openMenu();
    await screen.findByRole("menuitem", { name: "Rename" });
    expect(screen.getByRole("separator")).toBeTruthy();
    expect(
      (screen.getByRole("menuitem", { name: "Disabled" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeTruthy();
  });

  it("closes on Tab with focus restored", async () => {
    const trigger = openMenu();
    const rename = await screen.findByRole("menuitem", { name: "Rename" });
    fireEvent.keyDown(rename, { key: "Tab" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});
