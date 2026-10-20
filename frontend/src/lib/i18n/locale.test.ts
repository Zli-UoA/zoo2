import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vite-plus/test";

import { LOCALES, localeInitScript, matchLocale, readBrowserLocale, resolveLocale } from "./locale";
import { createI18n, resources } from "./index";

function flatten(value: object, prefix = ""): Record<string, string> {
  return Object.fromEntries(
    Object.entries(value).flatMap(([key, item]) => {
      const path = prefix ? `${prefix}.${key}` : key;
      return typeof item === "string" ? [[path, item]] : Object.entries(flatten(item, path));
    }),
  );
}

describe("locale resolution", () => {
  it.each([
    ["ja-JP", "ja"],
    ["en-GB", "en"],
    ["zh-TW", "zh-Hant"],
    ["zh-HK", "zh-Hant"],
    ["zh-MO", "zh-Hant"],
    ["zh-CN", "zh-Hans"],
    ["zh-SG", "zh-Hans"],
    ["zh-Hans-HK", "zh-Hans"],
    ["zh-Hant-CN", "zh-Hant"],
    ["yue-Hant-HK", "yue"],
    ["zh-yue-HK", "yue"],
    ["zh", "zh-Hans"],
    ["fr-FR", null],
  ])("matches %s to %s", (tag, expected) => expect(matchLocale(tag)).toBe(expected));

  it("prefers a saved selection, then the first supported browser language", () => {
    expect(resolveLocale("yue", ["en-US"])).toBe("yue");
    expect(resolveLocale("invalid", ["fr", "zh-TW", "en"])).toBe("zh-Hant");
    expect(resolveLocale(null, [])).toBe("ja");
    expect(resolveLocale(null, ["fr"])).toBe("ja");
  });

  it("uses browser languages even when storage is blocked", () => {
    vi.stubGlobal("window", {
      localStorage: {
        getItem() {
          throw new Error("blocked");
        },
      },
    });
    vi.stubGlobal("navigator", { languages: ["yue-HK"], language: "en" });
    try {
      expect(readBrowserLocale()).toBe("yue");
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("translations", () => {
  it.each(LOCALES)("has complete keys and matching placeholders in %s", (locale) => {
    const source = flatten(resources.ja.translation);
    const target = flatten(resources[locale].translation);
    expect(Object.keys(target).sort()).toEqual(Object.keys(source).sort());
    for (const [key, value] of Object.entries(target)) {
      expect(value.trim(), key).not.toBe("");
      expect(value.match(/{{\w+}}/g) ?? [], key).toEqual(source[key].match(/{{\w+}}/g) ?? []);
    }
  });

  it("handles English plurals and interpolation", () => {
    const t = createI18n("en").t;
    expect(t("messages.count", { count: 0 })).toBe("0 messages");
    expect(t("messages.count", { count: 1 })).toBe("1 message");
    expect(t("messages.count", { count: 2 })).toBe("2 messages");
    expect(t("threads.openWithCount", { count: 1 })).toBe("Open thread (1 reply)");
    expect(t("threads.openWithCount", { count: 2 })).toBe("Open thread (2 replies)");
    expect(t("members.already", { userId: "alice" })).toBe("Already a member: alice");
    expect(createI18n("yue").t("dates.yesterday")).toBe("尋日");
  });

  it("isolates instances and falls back to Japanese", async () => {
    const first = createI18n("en");
    const second = createI18n("yue");
    await first.changeLanguage("zh-Hans");
    expect(first.t("channels.title")).toBe("频道");
    expect(second.t("welcome.title")).toBe("歡迎嚟到 Zoo");
    await first.changeLanguage("unsupported");
    expect(first.t("channels.title")).toBe("チャンネル");
  });
});

describe("pre-hydration language initialization", () => {
  it.each([
    [null, ["en-US"], "en"],
    ["yue", ["en-US"], "yue"],
    ["invalid", ["fr", "zh-TW"], "zh-Hant"],
    [null, ["zh-Hans-HK"], "zh-Hans"],
    [null, ["yue-HK"], "yue"],
    [null, ["fr"], "ja"],
  ])(
    "sets html.lang before React for stored=%s and languages=%j",
    (stored, languages, expected) => {
      const document = { documentElement: { lang: "ja" } };
      runInNewContext(localeInitScript, {
        localStorage: { getItem: () => stored },
        navigator: { languages, language: languages[0] },
        document,
      });
      expect(document.documentElement.lang).toBe(expected);
    },
  );

  it("uses the browser language when reading storage throws", () => {
    const document = { documentElement: { lang: "ja" } };
    runInNewContext(localeInitScript, {
      get localStorage() {
        throw new Error("blocked");
      },
      navigator: { languages: ["en"], language: "en" },
      document,
    });
    expect(document.documentElement.lang).toBe("en");
  });
});
