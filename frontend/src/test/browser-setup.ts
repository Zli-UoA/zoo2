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

beforeAll(async () => {
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
