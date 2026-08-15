import { describe, expect, it } from "vitest";

import { uniqueChildName, validateFileName } from "./fileTreeNames";

describe("uniqueChildName", () => {
  it("keeps the original when free", () => {
    expect(uniqueChildName(["a.ts"], "b.ts")).toBe("b.ts");
  });

  it("inserts copy before the extension", () => {
    expect(uniqueChildName(["foo.ts"], "foo.ts")).toBe("foo copy.ts");
    expect(uniqueChildName(["foo.ts", "foo copy.ts"], "foo.ts")).toBe("foo copy 2.ts");
  });
});

describe("validateFileName", () => {
  it("rejects empty, dots, and path separators", () => {
    expect(validateFileName("")).not.toBeNull();
    expect(validateFileName("..")).not.toBeNull();
    expect(validateFileName("a/b")).not.toBeNull();
    expect(validateFileName("ok.ts")).toBeNull();
  });
});
