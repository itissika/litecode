import { describe, expect, it } from "vitest";

import {
  chipPath,
  dropCarriesMention,
  LITECODE_PATHS_MIME,
  LITECODE_SPAN_MIME,
  parsePathPayload,
  readCodeSpan,
  readLoosePaths,
  readTreePaths,
  spanFromSelection,
  writeCodeSpan,
} from "./dropPayload";
import { fakeTransfer } from "../test/fakeTransfer";

describe("drop payloads", () => {
  it("parses tree paths and ignores plain text when the mime is absent", () => {
    const dt = fakeTransfer();
    dt.setData("text/plain", "not/a/drag");
    expect(readTreePaths(dt)).toBeNull();
    dt.setData(LITECODE_PATHS_MIME, JSON.stringify(["src/a.rs", "docs"]));
    expect(readTreePaths(dt)).toEqual(["src/a.rs", "docs"]);
    expect(parsePathPayload("src/a.rs\nsrc/b.rs")).toEqual(["src/a.rs", "src/b.rs"]);
  });

  it("round-trips a code span and trims a caret on the following line", () => {
    const span = spanFromSelection("src/a.rs", {
      isEmpty: () => false,
      startLineNumber: 4,
      endLineNumber: 10,
      endColumn: 1,
    });
    expect(span).toEqual({ path: "src/a.rs", start: 4, end: 9 });
    const dt = fakeTransfer();
    writeCodeSpan(dt, span!);
    expect(readCodeSpan(dt)).toEqual(span);
    expect(spanFromSelection("src/a.rs", {
      isEmpty: () => true,
      startLineNumber: 1,
      endLineNumber: 1,
      endColumn: 1,
    })).toBeNull();
  });

  it("reads file URIs and absolute path lines, not ordinary code", () => {
    const uri = fakeTransfer();
    uri.setData("text/uri-list", "file:///E:/ws/src/a.rs\n# comment");
    expect(readLoosePaths(uri)).toEqual(["file:///E:/ws/src/a.rs"]);

    const code = fakeTransfer();
    code.setData("text/plain", "const x = 1;");
    expect(readLoosePaths(code)).toEqual([]);
    expect(dropCarriesMention(code)).toBe(false);

    const abs = fakeTransfer();
    abs.setData("text/plain", "C:\\outside\\a.ts");
    expect(readLoosePaths(abs)).toEqual(["C:\\outside\\a.ts"]);
  });

  it("relativizes a path inside the project and keeps one outside it", () => {
    expect(chipPath("E:\\ws\\src\\a.rs", "E:/ws")).toBe("src/a.rs");
    expect(chipPath("file:///E:/ws/src/a.rs", "e:/ws")).toBe("src/a.rs");
    expect(chipPath("C:/outside/a.ts", "E:/ws")).toBe("C:/outside/a.ts");
    expect(chipPath('C:/out/"quote".ts', "E:/ws")).toBe("quote.ts");
    expect(chipPath("src/a.rs", "E:/ws")).toBe("src/a.rs");
  });

  it("does not treat a code span as a foreign file drag", () => {
    const dt = fakeTransfer();
    dt.setData(LITECODE_SPAN_MIME, JSON.stringify({ path: "src/a.rs", start: 1, end: 2 }));
    dt.setData("text/plain", "code");
    expect(dropCarriesMention(dt)).toBe(true);
    expect(readCodeSpan(dt)?.path).toBe("src/a.rs");
  });
});
