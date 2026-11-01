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

  it("serializes reads and writes across caches sharing an OPFS connection", async () => {
    let active = 0;
    let peak = 0;
    const database = {
      async execute() {
        active++;
        peak = Math.max(peak, active);
        await Promise.resolve();
        active--;
        return [];
      },
    } as unknown as BrowserWASQLiteDatabase;
    const first = createComposeCache(database);
    const second = createComposeCache(database);
    await Promise.all([first.write("messages", "saved"), second.read("channels")]);
    expect(peak).toBe(1);
  });

  it("continues processing the connection after a failed statement", async () => {
    const execute = vi
      .fn()
      .mockResolvedValueOnce([])
      .mockRejectedValueOnce(new Error("write failed"))
      .mockResolvedValueOnce([{ value: "existing" }]);
    const cache = createComposeCache({ execute } as unknown as BrowserWASQLiteDatabase);
    await expect(cache.write("messages", "new")).rejects.toThrow("write failed");
    expect(await cache.read("messages")).toBe("existing");
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
