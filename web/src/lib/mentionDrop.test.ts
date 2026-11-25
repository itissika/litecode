import { beforeEach, describe, expect, it, vi } from "vitest";

import { fetchSymbolAt } from "../api/workspace";
import { LITECODE_PATHS_MIME, LITECODE_SPAN_MIME } from "./dropPayload";
import { mentionTextForDrop } from "./mentionDrop";
import { fakeTransfer } from "../test/fakeTransfer";
import { stubDesktopBridge } from "../test/desktopBridge";

vi.mock("../api/workspace", () => ({
  fetchSymbolAt: vi.fn(),
}));

const symbolAt = vi.mocked(fetchSymbolAt);

beforeEach(() => {
  symbolAt.mockReset();
});

describe("mentionTextForDrop", () => {
  it("turns file-tree paths into standard Markdown links", async () => {
    const dt = fakeTransfer();
    dt.setData(LITECODE_PATHS_MIME, JSON.stringify(["src/a.rs", "docs"]));
    dt.setData("text/plain", "src/a.rs\ndocs");
    await expect(mentionTextForDrop(dt, "E:/ws")).resolves.toBe(
      "[a.rs](src/a.rs) [docs](docs)",
    );
    expect(symbolAt).not.toHaveBeenCalled();
  });

  it("turns a code span into a symbol link, and a line-range link when lookup fails", async () => {
    symbolAt.mockResolvedValue({ chain: "Store.save" });
    const dt = fakeTransfer();
    dt.setData(
      LITECODE_SPAN_MIME,
      JSON.stringify({ path: "src/a.rs", start: 4, end: 9 }),
    );
    dt.setData("text/plain", "fn save() {}");
    await expect(mentionTextForDrop(dt, "")).resolves.toBe(
      "[Store.save](src/a.rs#Store.save)",
    );

    symbolAt.mockRejectedValue(new Error("offline"));
    await expect(mentionTextForDrop(dt, "")).resolves.toBe(
      "[a.rs:4-9](src/a.rs#L4-L9)",
    );
  });

  it("uses the desktop path for an OS file and a URI when there is no file", async () => {
    const file = new File(["x"], "a.ts", { type: "text/plain" });
    const files = fakeTransfer({ files: [file] });
    window.litecode = stubDesktopBridge({ getPathForFile: () => "E:\\ws\\src\\a.ts" });
    await expect(mentionTextForDrop(files, "E:/ws")).resolves.toBe("[a.ts](src/a.ts)");

    const uri = fakeTransfer();
    uri.setData("text/uri-list", "file:///C:/outside/a.ts");
    await expect(mentionTextForDrop(uri, "E:/ws")).resolves.toBe(
      "[a.ts](C:/outside/a.ts)",
    );
    Reflect.deleteProperty(window, "litecode");
  });
});
