import { afterAll, beforeAll, vi } from "vite-plus/test";

beforeAll(() => {
  vi.setSystemTime(new Date("2026-01-02T00:00:00.000Z"));
});

afterAll(() => {
  vi.useRealTimers();
});
