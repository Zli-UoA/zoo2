import { describe, expect, it } from "vite-plus/test";

import { normalizeUtcTimestamp, parseUtcTimestamp } from "./timestamps";

describe("normalizeUtcTimestamp", () => {
  it("converts Electric's space-separated form to UTC ISO", () => {
    expect(normalizeUtcTimestamp("2026-10-15 14:32:44.183")).toBe("2026-10-15T14:32:44.183Z");
  });

  it("normalizes REST's T-separated form to UTC ISO", () => {
    expect(normalizeUtcTimestamp("2026-10-15T14:32:44.183")).toBe("2026-10-15T14:32:44.183Z");
  });

  it("leaves values with an explicit offset or unknown shape unchanged", () => {
    expect(normalizeUtcTimestamp("2026-10-15 14:32:44.183+00")).toBe("2026-10-15 14:32:44.183+00");
    expect(normalizeUtcTimestamp("not-a-timestamp")).toBe("not-a-timestamp");
  });
});

describe("parseUtcTimestamp", () => {
  it("parses both Electric and REST forms as UTC", () => {
    const fromShape = parseUtcTimestamp("2026-10-15 14:32:44.183");
    const fromRest = parseUtcTimestamp("2026-10-15T14:32:44.183");
    expect(fromShape.toISOString()).toBe("2026-10-15T14:32:44.183Z");
    expect(fromRest.toISOString()).toBe(fromShape.toISOString());
  });
});
