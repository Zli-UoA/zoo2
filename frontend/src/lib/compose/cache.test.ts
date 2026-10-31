import { describe, it, expect, vi } from "vite-plus/test";
import type { BrowserWASQLiteDatabase } from "@tanstack/browser-db-sqlite-persistence";
import { createComposeCache } from "./cache";

describe("Compose offline cache", () => {
  it("keeps memory fallback local to one session", async () => {
    const first = createComposeCache(null);
    await first.write("messages", "alice");
    expect(await first.read("messages")).toBe("alice");
    expect(await createComposeCache(null).read("messages")).toBeNull();
  });

  it("reports a failed persistent write instead of claiming that it was saved", async () => {
    const failure = new DOMException("quota exceeded", "QuotaExceededError");
    const report = vi.fn();
    const execute = vi.fn().mockResolvedValueOnce([]).mockRejectedValueOnce(failure);
    const cache = createComposeCache({ execute } as unknown as BrowserWASQLiteDatabase, report);
    await expect(cache.write("messages", "new content")).rejects.toBe(failure);
    expect(report).toHaveBeenCalledWith(failure);
  });
});
