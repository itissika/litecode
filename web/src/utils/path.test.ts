import { describe, expect, it } from "vitest";

import { normalizeToolFilePath } from "../api/adapter";
import { joinWorkspacePath, remapPathPrefix, toWorkspacePath } from "./path";

describe("toWorkspacePath / normalizeToolFilePath", () => {
  it("strips LAP root with case-insensitive drive", () => {
    expect(toWorkspacePath("e:/proj/src/a.rs", "E:\\proj")).toBe("src/a.rs");
    expect(normalizeToolFilePath("E:\\proj\\src\\a.rs", "e:/proj")).toBe(
      "src/a.rs",
    );
  });

  it("keeps already-relative paths", () => {
    expect(toWorkspacePath("src/a.rs", "E:\\proj")).toBe("src/a.rs");
  });

  it("rejects verbatim forms", () => {
    expect(toWorkspacePath("\\\\?\\E:\\proj\\a.rs", "E:\\proj")).toBeNull();
    expect(normalizeToolFilePath("//?/E:/proj/a.rs", "E:/proj")).toBeNull();
  });
});

describe("joinWorkspacePath / remapPathPrefix", () => {
  it("joins and remaps descendants", () => {
    expect(joinWorkspacePath("src", "a.ts")).toBe("src/a.ts");
    expect(joinWorkspacePath("", "a.ts")).toBe("a.ts");
    expect(remapPathPrefix("src/a/x.ts", "src/a", "src/b")).toBe("src/b/x.ts");
    expect(remapPathPrefix("src/a.ts", "src/a", "src/b")).toBe("src/a.ts");
  });
});
