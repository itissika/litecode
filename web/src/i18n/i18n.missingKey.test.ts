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

  it("does not throw on missing keys (returns key)", () => {
    expect(() => i18n.t("does.not.exist.ever")).not.toThrow();
    expect(i18n.t("does.not.exist.ever")).toBe("does.not.exist.ever");
  });
});
