import { describe, expect, it } from "vitest";
import { createTranslate, normalizeLanguage, translate } from "@/i18n";
import { EN } from "@/i18n/dictionary";

describe("language normalization", () => {
  it("maps only the explicit English code to en", () => {
    expect(normalizeLanguage("en")).toBe("en");
    expect(normalizeLanguage("zh-CN")).toBe("zh-CN");
    expect(normalizeLanguage("system")).toBe("zh-CN");
    expect(normalizeLanguage(undefined)).toBe("zh-CN");
    expect(normalizeLanguage("")).toBe("zh-CN");
  });
});

describe("translate", () => {
  it("returns the Chinese source untouched in Chinese", () => {
    const t = createTranslate("zh-CN");
    expect(t("取消")).toBe("取消");
    expect(t("今日会话次数")).toBe("今日会话次数");
  });

  it("translates known keys in English and falls back to Chinese", () => {
    const t = createTranslate("en");
    expect(t("取消")).toBe("Cancel");
    expect(t("首页")).toBe("Home");
    expect(t("总使用次数")).toBe("Total uses");
    expect(t("未翻译的字符串")).toBe("未翻译的字符串");
  });

  it("substitutes {placeholders} in both languages", () => {
    const zh = createTranslate("zh-CN");
    const en = createTranslate("en");
    expect(zh("粘贴到 {app}", { app: "Safari" })).toBe("粘贴到 Safari");
    expect(en("粘贴到 {app}", { app: "Safari" })).toBe("Paste into Safari");
    expect(en("过去 {weeks} 周", { weeks: 26 })).toBe("Last 26 weeks");
    // A placeholder with no matching variable stays verbatim.
    expect(en("粘贴到 {app}")).toBe("Paste into {app}");
    // Nullish variables fall back to the raw placeholder rather than "null".
    expect(en("粘贴到 {app}", { app: null })).toBe("Paste into {app}");
  });

  it("context-free translate matches the provider behaviour", () => {
    expect(translate("en", "取消")).toBe("Cancel");
    expect(translate("zh-CN", "取消")).toBe("取消");
    expect(translate("en", "今日录音时长")).toBe("Mic time today");
  });
});

describe("dictionary consistency", () => {
  const placeholders = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

  it("every English value uses exactly the placeholders of its key", () => {
    const bad = Object.entries(EN).filter(
      ([key, value]) => placeholders(key).join(",") !== placeholders(value).join(","),
    );
    expect(bad).toEqual([]);
  });

  it("has no empty translations", () => {
    const empty = Object.entries(EN).filter(([, value]) => value.trim() === "");
    expect(empty).toEqual([]);
  });

  it("keeps the dashboard, shortcut and picker copy present", () => {
    // Spot-check the surfaces that were localized; a missing key silently
    // falls back to Chinese in English builds, so pin the important ones.
    for (const key of [
      "今日会话次数",
      "今日输入字数",
      "今日录音时长",
      "总使用次数",
      "总会话次数",
      "总输入字数",
      "总节省时间",
      "界面语言",
      "跟随系统",
      "选择要粘贴的版本",
      "Esc 取消",
      "识别中",
      "润色中",
      "按住说话",
      "免提模式",
    ]) {
      expect(EN[key], `missing EN translation for ${key}`).toBeTruthy();
      expect(EN[key], `EN translation of ${key} should be ASCII-ish`).not.toMatch(
        /[\u4e00-\u9fff]/,
      );
    }
  });
});
