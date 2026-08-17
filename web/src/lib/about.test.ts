import { describe, expect, it } from "vitest";

import {
  formatServerVersion,
  parseVersionChannel,
  shouldShowVersionChannel,
} from "./about";

describe("about version helpers", () => {
  it("parses known channels", () => {
    expect(parseVersionChannel("dev")).toBe("dev");
    expect(parseVersionChannel("nightly")).toBe("nightly");
    expect(parseVersionChannel("official")).toBe("official");
    expect(parseVersionChannel("")).toBeNull();
  });

  it("formats semver with a v prefix", () => {
    expect(formatServerVersion("0.1.4")).toBe("v0.1.4");
    expect(formatServerVersion("v0.1.4")).toBe("v0.1.4");
  });

  it("hides the channel tag for official builds only", () => {
    expect(shouldShowVersionChannel("official")).toBe(false);
    expect(shouldShowVersionChannel("dev")).toBe(true);
    expect(shouldShowVersionChannel("nightly")).toBe(true);
    expect(shouldShowVersionChannel(null)).toBe(false);
  });
});
