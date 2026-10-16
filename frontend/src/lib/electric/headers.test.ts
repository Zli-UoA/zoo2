import { describe, expect, it, vi } from "vite-plus/test";

import { createShapeFetch } from "./headers";

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
});
