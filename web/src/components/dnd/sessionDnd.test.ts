import { describe, expect, it } from "vitest";

import {
  parseSessionDrag,
  parseSessionFolderDrag,
} from "./sessionDnd";

describe("session drag payloads", () => {
  it("round-trips a session payload", () => {
    expect(parseSessionDrag(JSON.stringify({ kind: "session", id: "s1" }))).toEqual({
      kind: "session",
      id: "s1",
    });
  });

  it("round-trips a folder payload", () => {
    expect(
      parseSessionFolderDrag(
        JSON.stringify({ kind: "session-folder", id: "f1" }),
      ),
    ).toEqual({ kind: "session-folder", id: "f1" });
  });

  it("rejects the wrong kind, malformed JSON, and legacy raw ids", () => {
    expect(parseSessionDrag(JSON.stringify({ kind: "session-folder", id: "f1" }))).toBeNull();
    expect(parseSessionFolderDrag(JSON.stringify({ kind: "session", id: "s1" }))).toBeNull();
    expect(parseSessionDrag("{not json")).toBeNull();
    expect(parseSessionDrag("s1")).toBeNull();
    expect(parseSessionDrag("")).toBeNull();
  });
});
