import { describe, expect, it } from "vitest";

import { appDescription } from "./appText";

const app = {
  description: "Default text",
  description_i18n: {
    "zh-CN": "精确语言",
    zh: "基础语言",
    en: "English",
  },
};

describe("appDescription", () => {
  it("uses the exact locale tag first", () => {
    expect(appDescription(app, "zh-CN")).toBe("精确语言");
  });

  it("falls back to the base language", () => {
    expect(appDescription(app, "zh-Hans")).toBe("基础语言");
  });

  it("falls back to English, then to the default", () => {
    expect(appDescription(app, "fr")).toBe("English");
    expect(
      appDescription({ description: "Default text", description_i18n: {} }, "fr"),
    ).toBe("Default text");
  });
});
