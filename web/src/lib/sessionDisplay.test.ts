import { describe, expect, it } from "vitest";

import { sessionActivity, sessionTitle } from "./sessionDisplay";

describe("sessionTitle", () => {
  it("prefers the user title over the preview", () => {
    expect(sessionTitle({ title: "Planning", preview: "hello" })).toBe("Planning");
  });

  it("falls back to the user preview", () => {
    expect(sessionTitle({ title: null, preview: "  hello  " })).toBe("hello");
  });

  it("is empty when nothing names the session", () => {
    expect(sessionTitle({ title: "   ", preview: "  " })).toBe("");
    expect(sessionTitle(undefined)).toBe("");
  });
});

describe("sessionActivity", () => {
  it("prefers the assistant text", () => {
    expect(
      sessionActivity({ assistant_preview: " done ", preview: "do it" }),
    ).toBe("done");
  });

  it("falls back to the user preview", () => {
    expect(sessionActivity({ assistant_preview: "", preview: "do it" })).toBe(
      "do it",
    );
  });

  it("is empty for a control-only row", () => {
    expect(sessionActivity({ preview: "" })).toBe("");
    expect(sessionActivity(undefined)).toBe("");
  });
});
