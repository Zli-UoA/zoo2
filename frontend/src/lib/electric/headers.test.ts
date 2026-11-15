import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { setAuthorizedHandler, setUnauthorizedHandler } from "../auth";
import { OidcClient, type OidcEnvironment } from "../oidc/client";
import { createShapeFetch } from "./headers";

afterEach(() => {
  setAuthorizedHandler(null);
  setUnauthorizedHandler(null);
});

function captureFetch(status = 200) {
  const calls: RequestInit[] = [];
  const fetchImpl: typeof fetch = async (_input, init) => {
    calls.push(init ?? {});
    return new Response(null, { status });
  };
  return { calls, fetchImpl };
}

function headerValue(init: RequestInit | undefined, name: string): string | null {
  return new Headers(init?.headers).get(name);
}

describe("createShapeFetch", () => {
  it("does not attach auth headers when there is no token or dev user", async () => {
    const { calls, fetchImpl } = captureFetch();
    const shapeFetch = createShapeFetch({ getToken: () => null, fetch: fetchImpl });

    await shapeFetch("http://localhost:3000/v1/shape?table=users");

    expect(calls).toHaveLength(1);
    expect(headerValue(calls[0], "Authorization")).toBeNull();
    expect(headerValue(calls[0], "X-Dev-User")).toBeNull();
  });

  it("attaches a Bearer token resolved from the provider", async () => {
    const { calls, fetchImpl } = captureFetch();
    const shapeFetch = createShapeFetch({ getToken: () => "token-123", fetch: fetchImpl });

    await shapeFetch("http://localhost:3000/v1/shape?table=users");

    expect(headerValue(calls[0], "Authorization")).toBe("Bearer token-123");
  });

  it("attaches X-Dev-User only when a dev user is configured", async () => {
    const { calls, fetchImpl } = captureFetch();
    const shapeFetch = createShapeFetch({
      getToken: () => null,
      devUserId: "dev-user-1",
      fetch: fetchImpl,
    });

    await shapeFetch("http://localhost:8000/api/shapes/users");

    expect(headerValue(calls[0], "X-Dev-User")).toBe("dev-user-1");
    expect(headerValue(calls[0], "Authorization")).toBeNull();
  });

  it("preserves headers already set by the caller", async () => {
    const { calls, fetchImpl } = captureFetch();
    const shapeFetch = createShapeFetch({ getToken: () => null, fetch: fetchImpl });

    await shapeFetch("http://localhost:3000/v1/shape", {
      headers: { "Cache-Control": "no-cache" },
    });

    expect(headerValue(calls[0], "Cache-Control")).toBe("no-cache");
  });

  it("omits all auth headers when auth is disabled (direct mode)", async () => {
    const { calls, fetchImpl } = captureFetch();
    const shapeFetch = createShapeFetch({
      auth: false,
      getToken: () => "token-123",
      devUserId: "dev-user-1",
      fetch: fetchImpl,
    });

    await shapeFetch("http://localhost:3000/v1/shape?table=channels");

    expect(headerValue(calls[0], "Authorization")).toBeNull();
    expect(headerValue(calls[0], "X-Dev-User")).toBeNull();
  });

  it("notifies the unauthorized handler on a 401 response", async () => {
    const onUnauthorized = vi.fn();
    const { fetchImpl } = captureFetch(401);
    const shapeFetch = createShapeFetch({
      getToken: () => "token-123",
      onUnauthorized,
      fetch: fetchImpl,
    });

    await shapeFetch("http://localhost:8000/api/shapes/users");

    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });

  it("ignores a delayed 401 from before logout and handles a current-session 401", async () => {
    let finishOldFetch!: (response: Response) => void;
    let oldFetchStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      oldFetchStarted = resolve;
    });
    const oldResponse = new Promise<Response>((resolve) => {
      finishOldFetch = resolve;
    });
    const notify = vi.fn();
    setUnauthorizedHandler(notify);
    const shapeFetch = createShapeFetch({
      getToken: () => "access-token",
      fetch: vi
        .fn<typeof fetch>()
        .mockImplementationOnce(async () => {
          oldFetchStarted();
          return await oldResponse;
        })
        .mockResolvedValueOnce(new Response(null, { status: 401 })),
    });
    const storage = new Map<string, string>();
    const client = new OidcClient({
      issuer: "https://id.example",
      clientId: "zoo",
      environment: {
        origin: "https://app.example",
        href: "https://app.example/",
        assign: vi.fn(),
        replaceUrl: vi.fn(),
        storage: {
          getItem: (key) => storage.get(key) ?? null,
          setItem: (key, value) => void storage.set(key, value),
          removeItem: (key) => void storage.delete(key),
        },
        fetch: async () =>
          new Response(
            JSON.stringify({
              issuer: "https://id.example",
              authorization_endpoint: "https://id.example/authorize",
              token_endpoint: "https://id.example/token",
              end_session_endpoint: "https://id.example/logout",
            }),
            { headers: { "Content-Type": "application/json" } },
          ),
        now: () => 1,
      } satisfies OidcEnvironment,
    });

    const oldRequest = shapeFetch("https://api.example/shape");
    await started;
    await client.logout();
    finishOldFetch(new Response(null, { status: 401 }));
    await oldRequest;
    expect(notify).not.toHaveBeenCalled();

    await shapeFetch("https://api.example/shape");
    expect(notify).toHaveBeenCalledTimes(1);
  });

  it("does not notify on other statuses", async () => {
    const onUnauthorized = vi.fn();
    const { fetchImpl } = captureFetch(200);
    const shapeFetch = createShapeFetch({
      getToken: () => "token-123",
      onUnauthorized,
      fetch: fetchImpl,
    });

    await shapeFetch("http://localhost:8000/api/shapes/users");

    expect(onUnauthorized).not.toHaveBeenCalled();
  });

  it("notifies the authorized handler on success", async () => {
    const onAuthorized = vi.fn();
    const { fetchImpl } = captureFetch(200);
    const shapeFetch = createShapeFetch({
      getToken: () => "token-123",
      onAuthorized,
      fetch: fetchImpl,
    });

    await shapeFetch("http://localhost:8000/api/shapes/users");

    expect(onAuthorized).toHaveBeenCalledTimes(1);
  });

  it("ignores a delayed success from before logout when notifying authorization", async () => {
    let releaseOldResponse!: (response: Response) => void;
    let oldFetchStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      oldFetchStarted = resolve;
    });
    const oldResponse = new Promise<Response>((resolve) => {
      releaseOldResponse = resolve;
    });
    const onAuthorized = vi.fn();
    const shapeFetch = createShapeFetch({
      getToken: () => "access-token",
      onAuthorized,
      fetch: vi
        .fn<typeof fetch>()
        .mockImplementationOnce(async () => {
          oldFetchStarted();
          return await oldResponse;
        })
        .mockResolvedValueOnce(new Response(null, { status: 200 })),
    });
    const storage = new Map<string, string>();
    const client = new OidcClient({
      issuer: "https://id.example",
      clientId: "zoo",
      environment: {
        origin: "https://app.example",
        href: "https://app.example/",
        assign: vi.fn(),
        replaceUrl: vi.fn(),
        storage: {
          getItem: (key) => storage.get(key) ?? null,
          setItem: (key, value) => void storage.set(key, value),
          removeItem: (key) => void storage.delete(key),
        },
        fetch: async () =>
          new Response(
            JSON.stringify({
              issuer: "https://id.example",
              authorization_endpoint: "https://id.example/authorize",
              token_endpoint: "https://id.example/token",
              end_session_endpoint: "https://id.example/logout",
            }),
            { headers: { "Content-Type": "application/json" } },
          ),
        now: () => 1,
      } satisfies OidcEnvironment,
    });

    const oldRequest = shapeFetch("https://api.example/shape");
    await started;
    await client.logout();
    releaseOldResponse(new Response(null, { status: 200 }));
    await oldRequest;
    expect(onAuthorized).not.toHaveBeenCalled();

    await shapeFetch("https://api.example/shape");
    expect(onAuthorized).toHaveBeenCalledTimes(1);
  });

  it("does not notify the authorized handler when auth is disabled", async () => {
    const onAuthorized = vi.fn();
    const { fetchImpl } = captureFetch(200);
    const shapeFetch = createShapeFetch({ auth: false, onAuthorized, fetch: fetchImpl });

    await shapeFetch("http://localhost:3000/v1/shape?table=users");

    expect(onAuthorized).not.toHaveBeenCalled();
  });
});
