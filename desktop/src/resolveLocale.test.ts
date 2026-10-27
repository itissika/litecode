import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { resolveLocale } from "./resolveLocale";

describe("resolveLocale", () => {
  it("maps zh* to zh-CN", () => {
    assert.equal(resolveLocale("zh"), "zh-CN");
    assert.equal(resolveLocale("zh-CN"), "zh-CN");
    assert.equal(resolveLocale("zh-Hans-CN"), "zh-CN");
    assert.equal(resolveLocale("ZH-TW"), "zh-CN");
    assert.equal(resolveLocale("zh_CN"), "zh-CN");
  });

  it("maps en* to en", () => {
    assert.equal(resolveLocale("en"), "en");
    assert.equal(resolveLocale("en-US"), "en");
    assert.equal(resolveLocale("en-GB"), "en");
    assert.equal(resolveLocale("en_US"), "en");
  });

  it("falls back to en for unsupported tags", () => {
    assert.equal(resolveLocale("ja"), "en");
    assert.equal(resolveLocale("fr-FR"), "en");
    assert.equal(resolveLocale(""), "en");
    assert.equal(resolveLocale("  "), "en");
  });
});
