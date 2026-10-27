import { describe, expect, it } from "vitest";

import { resolveLocaleFromTags } from "./resolveLocale";

describe("resolveLocaleFromTags", () => {
  it("maps zh* to zh-CN", () => {
    expect(resolveLocaleFromTags(["zh"])).toBe("zh-CN");
    expect(resolveLocaleFromTags(["zh-CN"])).toBe("zh-CN");
    expect(resolveLocaleFromTags(["zh-Hans-CN"])).toBe("zh-CN");
    expect(resolveLocaleFromTags(["ZH-TW"])).toBe("zh-CN");
  });

  it("maps en* to en", () => {
    expect(resolveLocaleFromTags(["en"])).toBe("en");
    expect(resolveLocaleFromTags(["en-US"])).toBe("en");
    expect(resolveLocaleFromTags(["en-GB"])).toBe("en");
  });

  it("falls back to en for unsupported tags", () => {
    expect(resolveLocaleFromTags(["ja"])).toBe("en");
    expect(resolveLocaleFromTags(["fr-FR"])).toBe("en");
    expect(resolveLocaleFromTags([])).toBe("en");
    expect(resolveLocaleFromTags(["", "  "])).toBe("en");
  });

  it("picks the first matching tag in preference order", () => {
    expect(resolveLocaleFromTags(["ja", "zh-CN", "en"])).toBe("zh-CN");
    expect(resolveLocaleFromTags(["fr", "en-US"])).toBe("en");
  });
});
