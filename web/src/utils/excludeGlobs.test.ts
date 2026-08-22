import { describe, expect, it } from "vitest";

import { globsFromText, textFromGlobs } from "./excludeGlobs";

describe("excludeGlobs", () => {
  it("parses lines, comments, and duplicates", () => {
    expect(
      globsFromText("**/.git\n\n# skip\n**/node_modules\n**/.git\n"),
    ).toEqual(["**/.git", "**/node_modules"]);
  });

  it("round-trips a list", () => {
    const globs = ["**/.git", "**/vendor"];
    expect(globsFromText(textFromGlobs(globs))).toEqual(globs);
  });
});
