import { describe, expect, it } from "vite-plus/test";

import { nowIso } from "./time";
import { normalizeUtcTimestamp } from "./timestamps";

describe("nowIso", () => {
  it("returns a UTC ISO timestamp that timestamps.ts handles without normalization", () => {
    const value = nowIso();
    expect(value).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
    expect(normalizeUtcTimestamp(value)).toBe(value);
  });
});
