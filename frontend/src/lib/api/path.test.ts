import { describe, expect, it, vi } from "vite-plus/test";

import { createApiClient } from "./client";
import { apiPath, assertSafeApiPath, isUuid, UnsafeApiPathError } from "./path";

describe("isUuid", () => {
  it("accepts a UUID and rejects other shapes", () => {
    expect(isUuid("123e4567-e89b-12d3-a456-426614174000")).toBe(true);
    expect(isUuid("123e4567e89b12d3a456426614174000")).toBe(true);
    expect(isUuid("{123e4567-e89b-12d3-a456-426614174000}")).toBe(true);
    expect(isUuid("urn:uuid:123e4567-e89b-12d3-a456-426614174000")).toBe(true);
    expect(isUuid("123e4567-e89b-12d3-a456-426614174000/leave?")).toBe(false);
    expect(isUuid("123e4567e89b12d3a45642661417400")).toBe(false);
    expect(isUuid("{123e4567-e89b-12d3-a456-426614174000")).toBe(false);
    expect(isUuid("../apps/x")).toBe(false);
    expect(isUuid("")).toBe(false);
    expect(isUuid(undefined)).toBe(false);
  });
});

describe("apiPath", () => {
  it("keeps braces and colons of a UUID form inside one segment", () => {
    expect(apiPath`/api/channels/${"urn:uuid:x"}/view`).toBe("/api/channels/urn%3Auuid%3Ax/view");
  });

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
    "/api/home/https%3A%2F%2Fexample.test%2Fnotes%2F1/thread",
  ])("allows %s", (path) => {
    expect(() => assertSafeApiPath(path)).not.toThrow();
  });

  it.each([
    "api/channels",
    "//example.com/api",
    "/api/channels/../apps/x",
    "/api/channels/./x",
    "/api/channels/%2e%2e/x",
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
  it.each([
    "/api/channels/x/../y/view",
    "/api/channels/.\t./apps/x",
    "/api/channels/.\n./apps/x",
    "/api/channels/.\r./apps/x",
  ])("does not call fetch for an unsafe path %j", async (path) => {
    const fetchMock = vi.fn();
    const client = createApiClient({ baseUrl: "https://api.test", fetch: fetchMock });
    await expect(client.post(path)).rejects.toThrow(UnsafeApiPathError);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("sends an encoded path as is", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("{}", { status: 200 }));
    const client = createApiClient({ baseUrl: "https://api.test", fetch: fetchMock });
    await client.get(apiPath`/api/channels/${"a?c#d"}/view`);
    expect(fetchMock.mock.calls[0]![0]).toBe("https://api.test/api/channels/a%3Fc%23d/view");
  });
});
