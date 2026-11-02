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
    :root {
      --font-sans: "Geist Variable", "Noto Sans JP Variable", sans-serif;
      /* @apply font-sans inlines the production family, so set the actual family too. */
      font-family: "Geist Variable", "Noto Sans JP Variable", sans-serif;
    }
    *, *::before, *::after {
      animation: none !important;
      transition: none !important;
      caret-color: transparent !important;
    }
  `;
  document.head.append(style);
  await startMockServer();
  const fonts = await document.fonts.load('400 14px "Noto Sans JP Variable"', "日本語");
  if (!fonts.length) throw new Error("The fixed VRT Japanese font is not registered");
  await document.fonts.ready;
});
