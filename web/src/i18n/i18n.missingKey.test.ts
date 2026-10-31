import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json" with { type: "json" };
import zhCN from "./locales/zh-CN.json" with { type: "json" };

describe("i18n missing-key / skeleton keys", () => {
  beforeAll(async () => {
    if (!i18n.isInitialized) {
      await i18n.use(initReactI18next).init({
        resources: {
          en: { translation: en },
          "zh-CN": { translation: zhCN },
        },
        lng: "en",
        fallbackLng: "en",
        supportedLngs: ["en", "zh-CN"],
        interpolation: { escapeValue: false },
        returnNull: false,
        returnEmptyString: false,
        parseMissingKeyHandler: (key) => key,
      });
    } else {
      await i18n.changeLanguage("en");
    }
  });

  it("exposes settings.agents.title and hub.home.title in en", () => {
    expect(i18n.t("settings.agents.title")).toBe("Agents");
    expect(i18n.t("hub.home.title")).toBe("Workspace");
  });

  it("exposes the same keys in zh-CN", async () => {
    await i18n.changeLanguage("zh-CN");
    expect(i18n.t("settings.agents.title")).toBe("智能体");
    expect(i18n.t("hub.home.title")).toBe("工作区");
    await i18n.changeLanguage("en");
  });

  it("has browser panel strings in both locales", async () => {
    const keys = [
      "browser.title",
      "browser.unavailable",
      "browser.agentBusy",
      "browser.error.open",
    ];
    for (const key of keys) {
      expect(i18n.t(key)).not.toBe(key);
    }
    await i18n.changeLanguage("zh-CN");
    expect(i18n.t("browser.agentBusy")).toBe("智能体正在操作此页面…");
    for (const key of keys) {
      expect(i18n.t(key)).not.toBe(key);
    }
    await i18n.changeLanguage("en");
  });

  it("has explorer context-menu strings in both locales", async () => {
    const keys = [
      "explorer.newFile",
      "explorer.reveal",
      "explorer.copyPath",
      "explorer.delete",
      "explorer.revealFailed",
    ];
    for (const key of keys) {
      expect(i18n.t(key)).not.toBe(key);
    }
    await i18n.changeLanguage("zh-CN");
    expect(i18n.t("explorer.reveal")).toBe("在资源管理器中显示");
    for (const key of keys) {
      expect(i18n.t(key)).not.toBe(key);
    }
    // A count-bearing key must resolve through i18next plural lookup even
    // though only the base key exists.
    expect(i18n.t("explorer.deleteMany", { count: 3 })).toContain("3");
    expect(i18n.t("explorer.truncated", { count: 1000 })).toContain("1000");
    await i18n.changeLanguage("en");
    expect(i18n.t("explorer.deleteMany", { count: 3 })).toContain("items");
    expect(i18n.t("explorer.truncated", { count: 1000 })).toContain("1000");
  });

  it("does not throw on missing keys (returns key)", () => {
    expect(() => i18n.t("does.not.exist.ever")).not.toThrow();
    expect(i18n.t("does.not.exist.ever")).toBe("does.not.exist.ever");
  });
});
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

function flattenKeys(value: Record<string, unknown>, prefix = ""): string[] {
  return Object.entries(value).flatMap(([key, child]) => {
    const path = prefix + key;
    return child && typeof child === "object" && !Array.isArray(child)
      ? flattenKeys(child as Record<string, unknown>, path + ".")
      : [path];
  });
}

/** i18next plural suffixes are language-specific; compare the base key. */
function baseKey(key: string): string {
  return key.replace(/_(zero|one|two|few|many|other)$/, "");
}

const enKeys = new Set(flattenKeys(en).map(baseKey));
const zhKeys = new Set(flattenKeys(zhCN).map(baseKey));

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    if (!/\.(ts|tsx)$/.test(entry)) return [];
    if (/\.test\.(ts|tsx)$/.test(entry)) return [];
    return [path];
  });
}

describe("i18n locale parity", () => {
  it("keeps en and zh-CN at the same base-key set", () => {
    expect([...enKeys].filter((key) => !zhKeys.has(key))).toEqual([]);
    expect([...zhKeys].filter((key) => !enKeys.has(key))).toEqual([]);
  });

  it("only uses literal i18n keys that exist in both locales", () => {
    const root = join(process.cwd(), "src");
    const used = new Set<string>();
    for (const file of sourceFiles(root)) {
      const text = readFileSync(file, "utf8");
      for (const match of text.matchAll(/i18n\.t\(\s*["']([^"']+)["']/g)) {
        used.add(match[1]);
      }
    }

    const missing = [...used]
      .filter((key) => !enKeys.has(baseKey(key)) || !zhKeys.has(baseKey(key)))
      .sort();
    expect(missing).toEqual([]);
  });
});