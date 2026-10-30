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
