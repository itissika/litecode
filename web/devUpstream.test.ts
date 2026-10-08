// @vitest-environment node
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import {
  DEV_UPSTREAM_CLOSED,
  pointProxyAtDevUpstream,
  readDevUpstream,
} from "./devUpstream.ts";

const dirs: string[] = [];

afterEach(() => {
  for (const dir of dirs.splice(0)) {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

function tempFile(contents: string | null): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-upstream-"));
  dirs.push(dir);
  const file = path.join(dir, "upstream.txt");
  if (contents !== null) fs.writeFileSync(file, contents, "utf8");
  return file;
}

describe("readDevUpstream", () => {
  it("returns null when the file is missing or blank", () => {
    expect(readDevUpstream(tempFile(null))).toBeNull();
    expect(readDevUpstream(tempFile("  \n"))).toBeNull();
  });

  it("returns the loopback origin", () => {
    expect(readDevUpstream(tempFile("http://127.0.0.1:4312/"))).toBe(
      "http://127.0.0.1:4312",
    );
    expect(readDevUpstream(tempFile("http://localhost:9"))).toBe("http://localhost:9");
    expect(readDevUpstream(tempFile("http://[::1]:9/"))).toBe("http://[::1]:9");
  });

  it("rejects non-loopback and credentialed targets", () => {
    expect(readDevUpstream(tempFile("http://example.com:7483"))).toBeNull();
    expect(readDevUpstream(tempFile("http://user:pass@127.0.0.1:1"))).toBeNull();
    expect(readDevUpstream(tempFile("file:///c:/windows"))).toBeNull();
    expect(readDevUpstream(tempFile("not a url"))).toBeNull();
  });
});

describe("pointProxyAtDevUpstream", () => {
  it("points the proxy at the published origin", () => {
    const options: { target?: string } = {};
    expect(pointProxyAtDevUpstream(options, tempFile("http://127.0.0.1:5000/"))).toBe(
      true,
    );
    expect(options.target).toBe("http://127.0.0.1:5000");
  });

  it("falls back to a closed port when the host has not published yet", () => {
    const options: { target?: string } = { target: "http://127.0.0.1:1" };
    expect(pointProxyAtDevUpstream(options, tempFile(""))).toBe(false);
    expect(options.target).toBe(DEV_UPSTREAM_CLOSED);
  });
});
