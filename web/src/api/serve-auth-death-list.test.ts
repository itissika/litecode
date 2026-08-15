/**
 * Death list: persisted / settings-UI serve auth token must not return.
 * Product auth is host-injected via LITECODE_TOKEN (+ optional VITE_AUTH_TOKEN in dev).
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const WEB_SRC = join(__dirname, "..");

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

function walkTsFiles(dir: string, out: string[]): void {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const st = statSync(path);
    if (st.isDirectory()) {
      walkTsFiles(path, out);
    } else if (
      /\.(ts|tsx)$/.test(name) &&
      !name.endsWith(".test.ts") &&
      !name.endsWith(".test.tsx")
    ) {
      out.push(path);
    }
  }
}

describe("serve-auth settings death list", () => {
  it("bans settings-persisted serve auth needles under web/src (non-test)", () => {
    const files: string[] = [];
    walkTsFiles(WEB_SRC, files);
    const violations: string[] = [];
    for (const file of files) {
      const src = readFileSync(file, "utf8");
      const rel = relative(WEB_SRC, file);
      // Host injection helpers are allowed (env / preload), not settings persistence.
      if (rel === "api/auth.ts") continue;
      for (const needle of FORBIDDEN) {
        if (src.includes(needle)) {
          violations.push(`${rel}: forbidden \`${needle}\``);
        }
      }
    }
    expect(violations, violations.join("\n")).toEqual([]);
  });
});
