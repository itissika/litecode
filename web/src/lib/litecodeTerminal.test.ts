import { describe, expect, it } from "vitest";

import { trackCommandLine } from "./litecodeTerminal";

describe("trackCommandLine", () => {
  it("collects a submitted line as a command", () => {
    const typed = trackCommandLine("", "git status");
    expect(typed).toEqual({ line: "git status", commands: [] });
    expect(trackCommandLine(typed.line, "\r")).toEqual({
      line: "",
      commands: ["git status"],
    });
  });

  it("skips escape sequences so history recall cannot corrupt the line", () => {
    expect(trackCommandLine("", "ec\x1b[Dho").line).toBe("echo");
    expect(trackCommandLine("ls", "\x1b[A").line).toBe("ls");
  });

  it("applies backspace and ignores other control keys", () => {
    expect(trackCommandLine("gitx", "\x7f").line).toBe("git");
    expect(trackCommandLine("git", "\t").line).toBe("git");
  });

  it("clears the line on Ctrl-C / Ctrl-U and submits nothing", () => {
    expect(trackCommandLine("rm -rf /", "\x03").line).toBe("");
    expect(trackCommandLine("half", "\x15").commands).toEqual([]);
  });

  it("never reports an empty or whitespace-only line", () => {
    expect(trackCommandLine("", "\r").commands).toEqual([]);
    expect(trackCommandLine("   ", "\r").commands).toEqual([]);
  });

  it("reports every command a multi-line paste submits", () => {
    expect(trackCommandLine("", "echo a\r\necho b\r").commands).toEqual([
      "echo a",
      "echo b",
    ]);
  });
});
