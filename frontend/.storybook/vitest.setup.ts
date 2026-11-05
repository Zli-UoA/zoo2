import { configure } from "storybook/test";
import { afterAll, beforeAll, vi } from "vite-plus/test";

// The default 1s wait for findBy*/waitFor is too short on the ARC runners,
// where stories render more slowly than on GitHub-hosted runners.
configure({ asyncUtilTimeout: 5000 });

beforeAll(() => {
  vi.setSystemTime(new Date("2026-01-02T00:00:00.000Z"));
});

afterAll(() => {
  vi.useRealTimers();
});
