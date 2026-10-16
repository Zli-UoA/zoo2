import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import {
  getToken,
  notifyUnauthorized,
  resetTokenProvider,
  setTokenProvider,
  setUnauthorizedHandler,
} from "./auth";

afterEach(() => {
  resetTokenProvider();
  setUnauthorizedHandler(null);
});

describe("getToken", () => {
  it("returns null without a configured provider", async () => {
    expect(await getToken()).toBeNull();
  });

  it("returns the value from a synchronous provider", async () => {
    setTokenProvider(() => "token-123");
    expect(await getToken()).toBe("token-123");
  });

  it("awaits an asynchronous provider", async () => {
    setTokenProvider(async () => "async-token");
    expect(await getToken()).toBe("async-token");
  });

  it("can be reset back to the default", async () => {
    setTokenProvider(() => "token-123");
    resetTokenProvider();
    expect(await getToken()).toBeNull();
  });
});

describe("notifyUnauthorized", () => {
  it("invokes the registered handler", () => {
    const handler = vi.fn();
    setUnauthorizedHandler(handler);
    notifyUnauthorized();
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("does nothing without a handler", () => {
    expect(() => notifyUnauthorized()).not.toThrow();
  });
});
