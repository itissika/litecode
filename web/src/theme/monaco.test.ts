import { describe, expect, it } from "vitest";
import { toMonacoHex } from "./monaco";

describe("toMonacoHex", () => {
  it("passes through 6- and 8-digit hex", () => {
    expect(toMonacoHex("#ffffff")).toBe("#ffffff");
    expect(toMonacoHex("#AABBCCDD")).toBe("#AABBCCDD");
  });

  it("expands short hex", () => {
    expect(toMonacoHex("#abc")).toBe("#aabbcc");
    expect(toMonacoHex("#abcf")).toBe("#aabbccff");
  });

  it("converts rgba used by --_dk-line tokens", () => {
    expect(toMonacoHex("rgba(0, 0, 0, 0.05)")).toBe("#0000000d");
    expect(toMonacoHex("rgba(255, 255, 255, 0.10)")).toBe("#ffffff1a");
  });

  it("converts rgb", () => {
    expect(toMonacoHex("rgb(28, 28, 28)")).toBe("#1c1c1c");
  });

  it("falls back on garbage", () => {
    expect(toMonacoHex("not-a-color", "#112233")).toBe("#112233");
  });
});
