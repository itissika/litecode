import { describe, expect, it } from "vitest";

import {
  citationUrlTransform,
  classifyWorkspaceLink,
  parseFileCitation,
} from "./citationRef";

describe("parseFileCitation", () => {
  it("parses a file, a line, and a symbol", () => {
    expect(parseFileCitation("file:src/auth/validate.ts")).toEqual({
      kind: "file",
      path: "src/auth/validate.ts",
    });
    expect(parseFileCitation("file:src/auth/validate.ts#L42")).toEqual({
      kind: "line",
      path: "src/auth/validate.ts",
      line: 42,
    });
    expect(parseFileCitation("file:src/auth/validate.ts#Session.user")).toEqual({
      kind: "symbol",
      path: "src/auth/validate.ts",
      symbol: "Session.user",
    });
  });

  it("decodes the path and normalizes slashes", () => {
    expect(parseFileCitation("file:src/my%20file.ts")).toEqual({
      kind: "file",
      path: "src/my file.ts",
    });
    expect(parseFileCitation("file:src\\a.ts#L2")).toEqual({
      kind: "line",
      path: "src/a.ts",
      line: 2,
    });
  });

  it("rejects empty paths, traversal, and drive letters", () => {
    expect(parseFileCitation("file:")).toBeNull();
    expect(parseFileCitation("file:../secret")).toBeNull();
    expect(parseFileCitation("file:src/%2e%2e/secret")).toBeNull();
    expect(parseFileCitation("file:C:/Windows/note.txt")).toBeNull();
    expect(parseFileCitation("https://example.com")).toBeNull();
  });

  it("ignores a fragment that is not a line or a symbol", () => {
    expect(parseFileCitation("file:src/a.ts#not a symbol")).toEqual({
      kind: "file",
      path: "src/a.ts",
    });
    expect(parseFileCitation("file:src/a.ts#L0")).toEqual({
      kind: "file",
      path: "src/a.ts",
    });
  });
});

describe("citationUrlTransform", () => {
  it("keeps file links and still drops unsafe protocols", () => {
    expect(citationUrlTransform("file:src/a.ts#L42")).toBe("file:src/a.ts#L42");
    expect(citationUrlTransform("https://example.com")).toBe("https://example.com");
    expect(citationUrlTransform("javascript:alert(1)")).toBe("");
  });
});

describe("classifyWorkspaceLink", () => {
  it("chips a file link, at the line or symbol it names", () => {
    expect(classifyWorkspaceLink("file:src/a.ts")).toEqual({
      action: "chip",
      path: "src/a.ts",
      line: null,
      symbol: null,
    });
    expect(classifyWorkspaceLink("src/auth/validate.ts#L42")).toEqual({
      action: "chip",
      path: "src/auth/validate.ts",
      line: 42,
      symbol: null,
    });
    expect(classifyWorkspaceLink("src/session.rs#Session.user")).toEqual({
      action: "chip",
      path: "src/session.rs",
      line: null,
      symbol: "Session.user",
    });
    expect(classifyWorkspaceLink("./src/my%20file.ts")).toEqual({
      action: "chip",
      path: "src/my file.ts",
      line: null,
      symbol: null,
    });
    expect(classifyWorkspaceLink("src\\a.ts")).toEqual({
      action: "chip",
      path: "src/a.ts",
      line: null,
      symbol: null,
    });
  });

  it("turns a file-ish link we cannot open into text", () => {
    expect(classifyWorkspaceLink("../secret")).toEqual({ action: "text" });
    expect(classifyWorkspaceLink("/etc/hosts")).toEqual({ action: "text" });
    expect(classifyWorkspaceLink("file:../secret")).toEqual({ action: "text" });
    expect(classifyWorkspaceLink("file:C:/Windows/note.txt")).toEqual({
      action: "text",
    });
  });

  it("leaves a web link, an anchor, and a bare word alone", () => {
    expect(classifyWorkspaceLink("https://example.com")).toEqual({
      action: "link",
    });
    expect(classifyWorkspaceLink("mailto:someone@example.com")).toEqual({
      action: "link",
    });
    expect(classifyWorkspaceLink("//example.com/a.ts")).toEqual({
      action: "link",
    });
    expect(classifyWorkspaceLink("#section")).toEqual({ action: "link" });
    expect(classifyWorkspaceLink("here")).toEqual({ action: "link" });
    expect(classifyWorkspaceLink(undefined)).toEqual({ action: "link" });
  });
});
