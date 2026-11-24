import { cleanup } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { setupWorker } from "msw/browser";

const transparentPng = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  ),
  (char) => char.charCodeAt(0),
);

export const mockServer = setupWorker(
  http.get("https://www.gstatic.com/**", () =>
    HttpResponse.arrayBuffer(transparentPng.buffer as ArrayBuffer, {
      headers: { "Content-Type": "image/png" },
    }),
  ),
);
let started = false;

export async function startMockServer() {
  if (started) return;
  await mockServer.start({ onUnhandledFrame: "bypass" });
  started = true;
}

function ensureMatchMedia() {
  if (typeof window.matchMedia === "function") return;
  window.matchMedia = (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  });
}

function ensureObserver(name: "ResizeObserver" | "IntersectionObserver") {
  if (name in window) return;
  class Observer {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(window, name, { value: Observer, configurable: true });
}

// ブラウザーテストでアニメーション完了待ちによるタイムアウトを防ぐため、VRT（vrt-setup.ts）と同じ
// CSS 停止に加え、Base UI の終了アニメーション・animationFrame 待機（BASE_UI_ANIMATIONS_DISABLED）も抑止する。
const DISABLE_ANIMATIONS_ATTRIBUTE = "data-test-disable-animations";
function disableAnimations() {
  (globalThis as { BASE_UI_ANIMATIONS_DISABLED?: boolean }).BASE_UI_ANIMATIONS_DISABLED = true;
  const style = document.createElement("style");
  style.setAttribute(DISABLE_ANIMATIONS_ATTRIBUTE, "");
  style.textContent = "*,*::before,*::after{animation:none!important;transition:none!important}";
  document.head.append(style);
}

/**
 * アニメーションそのものを検証するテストだけが使う。上の全体の停止を一時的に外し、元に戻す関数を返す。
 * 時刻を止めて不透明度を読むなど、実時間を待たない形で使う（待つとテストが揺れる）。
 */
export function enableAnimations(): () => void {
  // セットアップが複数回走ると、同じ停止のスタイルが重ねて入る。すべて外す。
  const styles = [
    ...document.querySelectorAll<HTMLStyleElement>(`style[${DISABLE_ANIMATIONS_ATTRIBUTE}]`),
  ];
  for (const style of styles) style.disabled = true;
  return () => {
    for (const style of styles) style.disabled = false;
  };
}

beforeAll(async () => {
  disableAnimations();
  ensureMatchMedia();
  ensureObserver("ResizeObserver");
  ensureObserver("IntersectionObserver");
  if (typeof window.scrollTo !== "function") {
    Object.defineProperty(window, "scrollTo", { value: () => {}, configurable: true });
  }
  localStorage.setItem("zoo-locale", "ja");
  await document.fonts.ready;
});

afterEach(() => {
  cleanup();
  mockServer.resetHandlers();
  localStorage.clear();
  localStorage.setItem("zoo-locale", "ja");
});

afterAll(() => {
  if (started) void mockServer.stop();
});
