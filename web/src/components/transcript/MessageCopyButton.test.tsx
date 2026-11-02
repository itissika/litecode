import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MessageCopyButton } from "./MessageCopyButton";

function stubClipboard(writeText: ReturnType<typeof vi.fn>) {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
}

afterEach(() => {
  Reflect.deleteProperty(navigator, "clipboard");
});

describe("MessageCopyButton", () => {
  it("copies the source text as given, citations included", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    stubClipboard(writeText);
    const text = 'Look [@ file="src/a.ts" lines="12-30"] and stop.';
    render(<MessageCopyButton text={text} />);

    fireEvent.click(screen.getByRole("button", { name: "Copy message" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(text));
    expect(await screen.findByRole("button", { name: "Copied" })).toBeTruthy();
  });

  it("stays quiet when the clipboard refuses", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    stubClipboard(writeText);
    render(<MessageCopyButton text="hi" />);

    fireEvent.click(screen.getByRole("button", { name: "Copy message" }));

    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: "Copy message" })).toBeTruthy();
  });
});
