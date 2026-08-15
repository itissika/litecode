import { describe, expect, it } from "vitest";

import { tokenFromPageUrl } from "./auth";

describe("tokenFromPageUrl", () => {
  it("reads token query param", () => {
    expect(tokenFromPageUrl("?token=abc123")).toBe("abc123");
    expect(tokenFromPageUrl("?foo=1&token=xyz")).toBe("xyz");
  });

  it("returns undefined when missing or empty", () => {
    expect(tokenFromPageUrl("")).toBeUndefined();
    expect(tokenFromPageUrl("?foo=1")).toBeUndefined();
    expect(tokenFromPageUrl("?token=")).toBeUndefined();
  });
});
