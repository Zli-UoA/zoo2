import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { setAppBadge } from "./badge";

afterEach(() => vi.unstubAllGlobals());

describe("setAppBadge", () => {
  it("sets the count when positive", () => {
    const setter = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { setAppBadge: setter, clearAppBadge: vi.fn() });
    setAppBadge(3);
    expect(setter).toHaveBeenCalledWith(3);
  });

  it("clears the badge at zero or below", () => {
    const clear = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { setAppBadge: vi.fn(), clearAppBadge: clear });
    setAppBadge(0);
    expect(clear).toHaveBeenCalled();
  });

  it("does nothing without the API", () => {
    vi.stubGlobal("navigator", {});
    expect(() => setAppBadge(2)).not.toThrow();
  });
});

describe("setAppBadge errors", () => {
  it("clears for negative counts", () => {
    const clear = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { setAppBadge: vi.fn(), clearAppBadge: clear });
    setAppBadge(-1);
    expect(clear).toHaveBeenCalled();
  });

  it("swallows rejections", async () => {
    const setter = vi.fn().mockRejectedValue(new Error("denied"));
    vi.stubGlobal("navigator", { setAppBadge: setter, clearAppBadge: vi.fn() });
    expect(() => setAppBadge(1)).not.toThrow();
    await Promise.resolve();
  });
});
