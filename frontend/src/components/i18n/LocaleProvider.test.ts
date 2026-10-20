// @vitest-environment happy-dom
import { act, createElement, useLayoutEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { useTranslation } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ThemeProvider, useTheme } from "../theme/ThemeProvider";
import { composerSendHint } from "../chat/messages/Composer";
import { LocaleProvider, useLocale } from "./LocaleProvider";

async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}

let root: Root;
let element: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  element = document.createElement("div");
  document.body.append(element);
  root = createRoot(element);
});
afterEach(async () => {
  await flush(() => root.unmount());
  element.remove();
  vi.unstubAllGlobals();
});

describe("LocaleProvider", () => {
  it("starts in the saved language and keeps every committed render consistent when switching", async () => {
    localStorage.setItem("zoo-locale", "en");
    const commits: Array<{ locale: string; language: string; title: string; hint: string }> = [];
    let value: ReturnType<typeof useLocale>;
    function Probe() {
      const context = useLocale();
      const { t, i18n } = useTranslation();
      value = context;
      useLayoutEffect(() => {
        commits.push({
          locale: context.locale,
          language: i18n.language,
          title: t("welcome.title"),
          hint: composerSendHint("mac", t),
        });
      });
      return null;
    }
    await flush(() =>
      root.render(createElement(LocaleProvider, { children: createElement(Probe) })),
    );
    expect(commits.length).toBeGreaterThan(0);
    expect(
      commits.every(
        (entry) =>
          entry.locale === "en" && entry.language === "en" && entry.title === "Welcome to Zoo",
      ),
    ).toBe(true);
    commits.length = 0;
    await flush(() => value.setLocale("yue"));
    expect(commits.length).toBeGreaterThan(0);
    expect(
      commits.every(
        (entry) =>
          entry.locale === "yue" &&
          entry.language === "yue" &&
          entry.title === "歡迎嚟到 Zoo" &&
          entry.hint === "⌘（Cmd）+Enter 傳送 / Enter 換行",
      ),
    ).toBe(true);
    expect(document.documentElement.lang).toBe("yue");
  });

  it("keeps the context value and setter stable across parent renders", async () => {
    const values: Array<ReturnType<typeof useLocale>> = [];
    function Probe() {
      values.push(useLocale());
      return null;
    }
    const render = () =>
      root.render(createElement(LocaleProvider, { children: createElement(Probe) }));
    await flush(render);
    await flush(render);
    expect(values.length).toBe(2);
    expect(values[1]).toBe(values[0]);
    expect(values[1].setLocale).toBe(values[0].setLocale);
  });

  it("synchronizes locale and theme after storage changes and localStorage.clear", async () => {
    localStorage.setItem("zoo-locale", "yue");
    localStorage.setItem("zoo-theme", "dark");
    let state: { locale: string; theme: string };
    function Probe() {
      state = { locale: useLocale().locale, theme: useTheme().theme };
      return null;
    }
    await flush(() =>
      root.render(
        createElement(LocaleProvider, {
          children: createElement(ThemeProvider, { children: createElement(Probe) }),
        }),
      ),
    );
    expect(state!).toEqual({ locale: "yue", theme: "dark" });
    await flush(() => {
      localStorage.setItem("zoo-locale", "zh-Hans");
      window.dispatchEvent(new StorageEvent("storage", { key: "zoo-locale" }));
    });
    expect(state!).toEqual({ locale: "zh-Hans", theme: "dark" });
    await flush(() => {
      localStorage.clear();
      window.dispatchEvent(new StorageEvent("storage", { key: null }));
    });
    expect(state!.locale).toBe("en");
    expect(state!.theme).toBe("system");
  });
});
