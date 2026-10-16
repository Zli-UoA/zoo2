import { describe, expect, it, vi } from "vite-plus/test";

import {
  ApiError,
  buildAuthHeaders,
  createApiClient,
  createConfiguredApiClient,
  unwrapEnvelope,
  type FetchLike,
} from "./client";
import { resolveConfig } from "../config";

function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

describe("buildAuthHeaders", () => {
  it("adds Bearer and X-Dev-User when provided", async () => {
    const headers = await buildAuthHeaders({ getToken: () => "token-abc", devUserId: "dev-1" });
    expect(headers.get("Authorization")).toBe("Bearer token-abc");
    expect(headers.get("X-Dev-User")).toBe("dev-1");
  });

  it("returns no auth headers without credentials", async () => {
    const headers = await buildAuthHeaders({ getToken: () => null });
    expect(headers.has("Authorization")).toBe(false);
    expect(headers.has("X-Dev-User")).toBe(false);
  });
});

describe("unwrapEnvelope", () => {
  it("extracts data and txid", () => {
    expect(unwrapEnvelope({ data: { id: "1" }, txid: "99" })).toEqual({
      data: { id: "1" },
      txid: "99",
    });
  });

  it("throws when the envelope is malformed", () => {
    expect(() => unwrapEnvelope({ data: {} })).toThrow();
    expect(() => unwrapEnvelope({ txid: "1" })).toThrow();
    expect(() => unwrapEnvelope(null)).toThrow();
  });

  it("rejects an envelope whose txid is not a string", () => {
    expect(() => unwrapEnvelope({ data: {}, txid: 1 })).toThrow();
  });
});

describe("createApiClient", () => {
  it("injects Bearer, X-Dev-User, and JSON headers on writes", async () => {
    const fetchMock = vi.fn<FetchLike>(async () =>
      jsonResponse({ data: { id: "m1" }, txid: "100" }),
    );
    const client = createApiClient({
      baseUrl: "http://localhost:8000",
      devUserId: "00000000-0000-0000-0000-000000000001",
      getToken: () => "token-abc",
      fetch: fetchMock,
    });

    const result = await client.post("/api/messages", { content: "hi" });

    expect(result).toEqual({ data: { id: "m1" }, txid: "100" });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("http://localhost:8000/api/messages");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe(JSON.stringify({ content: "hi" }));
    const headers = new Headers(init?.headers);
    expect(headers.get("Authorization")).toBe("Bearer token-abc");
    expect(headers.get("X-Dev-User")).toBe("00000000-0000-0000-0000-000000000001");
    expect(headers.get("Content-Type")).toBe("application/json");
    expect(headers.get("Accept")).toBe("application/json");
  });

  it("returns the raw body for read endpoints", async () => {
    const users = [{ id: "u1", user_name: "alice", display_name: "Alice" }];
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse(users));
    const client = createApiClient({ baseUrl: "http://localhost:8000", fetch: fetchMock });

    await expect(client.get("/api/users")).resolves.toEqual(users);
  });

  it("omits auth headers when no token or dev user is configured", async () => {
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse({ data: {}, txid: "1" }));
    const client = createApiClient({ baseUrl: "http://localhost:8000", fetch: fetchMock });

    await client.post("/api/messages", {});

    const [, init] = fetchMock.mock.calls[0]!;
    const headers = new Headers(init?.headers);
    expect(headers.has("Authorization")).toBe(false);
    expect(headers.has("X-Dev-User")).toBe(false);
  });

  it("uses PATCH and DELETE and unwraps the envelope for each", async () => {
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse({ data: { ok: true }, txid: "7" }));
    const client = createApiClient({ baseUrl: "http://localhost:8000", fetch: fetchMock });

    await expect(client.patch("/api/messages/m1", { content: "edited" })).resolves.toEqual({
      data: { ok: true },
      txid: "7",
    });
    await expect(client.delete("/api/messages/m1")).resolves.toEqual({
      data: { ok: true },
      txid: "7",
    });

    expect(fetchMock.mock.calls[0]?.[1]?.method).toBe("PATCH");
    expect(fetchMock.mock.calls[1]?.[1]?.method).toBe("DELETE");
  });

  it("returns null for an empty read body", async () => {
    const fetchMock = vi.fn<FetchLike>(async () => new Response("", { status: 200 }));
    const client = createApiClient({ baseUrl: "http://localhost:8000", fetch: fetchMock });

    await expect(client.get("/api/ping")).resolves.toBeNull();
  });

  it("keeps a non-JSON error body as text", async () => {
    const fetchMock = vi.fn<FetchLike>(async () => new Response("boom", { status: 500 }));
    const client = createApiClient({ baseUrl: "http://localhost:8000", fetch: fetchMock });

    await expect(client.get("/api/boom")).rejects.toMatchObject({ status: 500, body: "boom" });
  });

  it("forwards the abort signal", async () => {
    const controller = new AbortController();
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse({ data: {}, txid: "1" }));
    const client = createApiClient({ baseUrl: "http://localhost:8000", fetch: fetchMock });

    await client.post("/api/messages", {}, { signal: controller.signal });

    expect(fetchMock.mock.calls[0]?.[1]?.signal).toBe(controller.signal);
  });

  it("throws ApiError with the status and body on failure", async () => {
    const fetchMock = vi.fn<FetchLike>(
      async () => new Response(JSON.stringify({ error: "forbidden" }), { status: 403 }),
    );
    const client = createApiClient({ baseUrl: "http://localhost:8000", fetch: fetchMock });

    await expect(client.get("/api/secrets")).rejects.toMatchObject({
      name: "ApiError",
      status: 403,
      body: { error: "forbidden" },
    });
    await expect(client.get("/api/secrets")).rejects.toBeInstanceOf(ApiError);
  });
});

describe("createConfiguredApiClient", () => {
  it("uses the API base URL and dev user from the app config", async () => {
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse({ data: {}, txid: "1" }));
    const config = resolveConfig(
      { VITE_SHAPE_MODE: "proxy" },
      { apiBaseUrl: "http://api.test", devUserId: "dev-user" },
    );
    const client = createConfiguredApiClient(config, { fetch: fetchMock });

    await client.get("/api/channels");

    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("http://api.test/api/channels");
    expect(new Headers(init?.headers).get("X-Dev-User")).toBe("dev-user");
  });
});
