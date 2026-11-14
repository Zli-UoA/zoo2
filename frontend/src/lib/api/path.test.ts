import { describe, expect, it, vi } from "vite-plus/test";

import { createApiClient } from "./client";
import { apiPath, assertSafeApiPath, isUuid, UnsafeApiPathError } from "./path";

describe("isUuid", () => {
  it("accepts a UUID and rejects other shapes", () => {
    expect(isUuid("123e4567-e89b-12d3-a456-426614174000")).toBe(true);
    expect(isUuid("123e4567-e89b-12d3-a456-426614174000/leave?")).toBe(false);
    expect(isUuid("../apps/x")).toBe(false);
    expect(isUuid("")).toBe(false);
    expect(isUuid(undefined)).toBe(false);
  });
});

describe("apiPath", () => {
  it("encodes interpolated values as a single path segment", () => {
    expect(apiPath`/api/channels/${"abc"}/view`).toBe("/api/channels/abc/view");
    expect(apiPath`/api/channels/${"a/b?c#d"}/view`).toBe("/api/channels/a%2Fb%3Fc%23d/view");
    expect(apiPath`/api/channels/${".."}/view`).toBe("/api/channels/../view");
  });
});

describe("assertSafeApiPath", () => {
  it.each([
    "/api/channels/abc/view",
    "/api/channels/abc/read-state?after=1",
    "/api/threads?cursor=a/b",
    "/api/e2ee/devices/a%20b",
  ])("allows %s", (path) => {
    expect(() => assertSafeApiPath(path)).not.toThrow();
  });

  it.each([
    "api/channels",
    "//example.com/api",
    "/api/channels/../apps/x",
    "/api/channels/./x",
    "/api/channels/%2e%2e/x",
    "/api/channels/a%2Fb/view",
    "/api/channels/a%5Cb/view",
    "/api/channels/a\\b/view",
    "/api/channels/a#b",
    "/api/channels/a?x=1#b",
    "/api//channels",
    "/api/channels/.\t./apps/x",
    "/api/channels/.\n./apps/x",
    "/api/channels/.\r./apps/x",
  ])("rejects %s", (path) => {
    expect(() => assertSafeApiPath(path)).toThrow(UnsafeApiPathError);
  });
});

describe("createApiClient path check", () => {
  it("does not call fetch for an unsafe path", async () => {
    const fetchMock = vi.fn();
    const client = createApiClient({ baseUrl: "https://api.test", fetch: fetchMock });
    await expect(client.post("/api/channels/x/../y/view")).rejects.toThrow(UnsafeApiPathError);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("sends an encoded path as is", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("{}", { status: 200 }));
    const client = createApiClient({ baseUrl: "https://api.test", fetch: fetchMock });
    await client.get(apiPath`/api/channels/${"a?c#d"}/view`);
    expect(fetchMock.mock.calls[0]![0]).toBe("https://api.test/api/channels/a%3Fc%23d/view");
  });
});
