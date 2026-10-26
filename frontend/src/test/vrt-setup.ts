import { beforeAll, vi } from "vite-plus/test";
import { http, HttpResponse } from "msw";

import { mockServer, startMockServer } from "./browser-setup";

const transparentPng = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
  ),
  (char) => char.charCodeAt(0),
);

beforeAll(async () => {
  vi.setSystemTime(new Date("2026-01-02T00:00:00.000Z"));
  const style = document.createElement("style");
  style.textContent = `
    *, *::before, *::after {
      animation: none !important;
      transition: none !important;
      caret-color: transparent !important;
    }
  `;
  document.head.append(style);
  mockServer.use(
    http.get("https://www.gstatic.com/**", () =>
      HttpResponse.arrayBuffer(transparentPng.buffer as ArrayBuffer, {
        headers: { "Content-Type": "image/png" },
      }),
    ),
  );
  await startMockServer();
  await document.fonts.ready;
});
