import "@fontsource-variable/noto-sans-jp";
import { beforeAll, beforeEach, vi } from "vite-plus/test";

import { storybookApiHandlers } from "../../.storybook/mock-api";
import { mockServer, startMockServer } from "./browser-setup";

beforeEach(() => {
  mockServer.use(...storybookApiHandlers);
});

beforeAll(async () => {
  vi.setSystemTime(new Date("2026-01-02T00:00:00.000Z"));
  const style = document.createElement("style");
  style.textContent = `
    /* Keep Japanese glyphs independent from OS fallback fonts. */
    :root { --font-sans: "Geist Variable", "Noto Sans JP Variable", sans-serif; }
    *, *::before, *::after {
      animation: none !important;
      transition: none !important;
      caret-color: transparent !important;
    }
  `;
  document.head.append(style);
  await startMockServer();
  await document.fonts.ready;
});
