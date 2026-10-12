/**
 * Death list: persisted / settings-UI serve auth token must not return.
 * Product auth is host-injected via LITECODE_TOKEN (+ optional VITE_AUTH_TOKEN in dev).
 */
import { describe, expect, it } from "vitest";
import { collectTsFiles, scanNeedles } from "../test/deathListGate";

const FORBIDDEN = [
  "putAuth",
  "saveAuth",
  "AuthWriteResponse",
  "has_auth_token",
  "/api/settings/auth",
  "auth.token",
  "Auth token (dev only)",
  "Save auth",
] as const;

describe("serve-auth settings death list", () => {
  it("bans settings-persisted serve auth needles under web/src (non-test)", () => {
    const files = collectTsFiles();
    // Host injection helpers are allowed (env / preload), not settings persistence.
    const violations = scanNeedles(files, FORBIDDEN, undefined, (rel) => rel === "api/auth.ts");
    expect(violations, violations.join("\n")).toEqual([]);
  });
});
