import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import type { FetchLike } from "./api/client";
import { resetTokenProvider, setTokenProvider, setUnauthorizedHandler } from "./auth";
import { resolveConfig } from "./config";
import { createCurrentUserStore, resolveCurrentUser, type CurrentUserState } from "./currentUser";
import { OidcClient } from "./oidc/client";

const DEV_CONFIG = resolveConfig({
  DEV: true,
  VITE_SHAPE_MODE: "proxy",
  VITE_DEV_USER_ID: "dev-user-1",
});

const OIDC_CONFIG = resolveConfig({
  DEV: true,
  VITE_SHAPE_MODE: "proxy",
  VITE_OIDC_ISSUER: "https://id.example",
  VITE_OIDC_CLIENT_ID: "zoo",
});

const DIRECT_DEV_CONFIG = resolveConfig({
  DEV: true,
  VITE_SHAPE_MODE: "direct",
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function userResponse(id: string, isAdmin = false): Record<string, unknown> {
  return {
    id,
    user_name: "smoke",
    display_name: "Smoke User",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: null,
    is_admin: isAdmin,
  };
}

afterEach(() => {
  resetTokenProvider();
  setUnauthorizedHandler(null);
});

describe("resolveCurrentUser", () => {
  it("resolves the id and is_admin from GET /api/users/me with X-Dev-User in dev", async () => {
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse(userResponse("dev-user-1", true)));
    const resolved = await resolveCurrentUser(DEV_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: "dev-user-1", isAdmin: true, error: null });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("http://localhost:8000/api/users/me");
    expect(new Headers(init?.headers).get("X-Dev-User")).toBe("dev-user-1");
  });

  it("falls back to VITE_DEV_USER_ID when the dev /me request fails", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ error: "boom" }, 500));
    const resolved = await resolveCurrentUser(DEV_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: "dev-user-1", isAdmin: false, error: null });
  });

  it("keeps the current user unresolved in dev direct mode without VITE_DEV_USER_ID", async () => {
    const fetchMock = vi.fn();
    const resolved = await resolveCurrentUser(DIRECT_DEV_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: null, isAdmin: false, error: null });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("resolves the id and is_admin from GET /api/users/me with a Bearer token when OIDC is configured", async () => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse(userResponse("user-1", true)));
    const resolved = await resolveCurrentUser(OIDC_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: "user-1", isAdmin: true, error: null });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("http://localhost:8000/api/users/me");
    expect(new Headers(init?.headers).get("Authorization")).toBe("Bearer token-123");
  });

  it("treats a missing is_admin field as false for backward compatibility", async () => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi.fn<FetchLike>(async () =>
      jsonResponse({
        id: "user-1",
        user_name: "smoke",
        display_name: "Smoke User",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: null,
      }),
    );
    const resolved = await resolveCurrentUser(OIDC_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: "user-1", isAdmin: false, error: null });
  });

  it("returns null and leaves re-login to the unauthorized handler on 401", async () => {
    const onUnauthorized = vi.fn();
    setUnauthorizedHandler(onUnauthorized);
    const fetchMock = vi.fn(async () => jsonResponse({ error: "unauthorized" }, 401));
    const resolved = await resolveCurrentUser(OIDC_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: null, isAdmin: false, error: null });
    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });

  it("reports other failures without a current user", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ error: "boom" }, 500));
    const resolved = await resolveCurrentUser(OIDC_CONFIG, { fetch: fetchMock });
    expect(resolved.currentUserId).toBeNull();
    expect(resolved.isAdmin).toBe(false);
    expect(resolved.error).toMatch(/500/);
  });
});

describe("createCurrentUserStore", () => {
  it("is already resolved from VITE_DEV_USER_ID in dev mode", () => {
    const store = createCurrentUserStore(DEV_CONFIG);
    expect(store.getState()).toEqual({
      currentUserId: "dev-user-1",
      isLoading: false,
      isAdmin: false,
      error: null,
    });
  });

  it("updates is_admin in dev without clearing the current user", async () => {
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse(userResponse("dev-user-1", true)));
    const store = createCurrentUserStore(DEV_CONFIG, { fetch: fetchMock });

    await store.ensureLoaded();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(store.getState()).toEqual({
      currentUserId: "dev-user-1",
      isLoading: false,
      isAdmin: true,
      error: null,
    });
  });

  it("resolves once and notifies subscribers", async () => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse(userResponse("user-1", true)));
    const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });
    const states: CurrentUserState[] = [];
    store.subscribe(() => states.push(store.getState()));
    expect(store.getState()).toEqual({
      currentUserId: null,
      isLoading: true,
      isAdmin: false,
      error: null,
    });

    await store.ensureLoaded();
    await store.ensureLoaded();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(store.getState()).toEqual({
      currentUserId: "user-1",
      isLoading: false,
      isAdmin: true,
      error: null,
    });
    expect(states.at(-1)?.currentUserId).toBe("user-1");
  });

  it("ignores a late current-user response after logout and resolves the next session", async () => {
    let finishOldRequest!: (response: Response) => void;
    let oldRequestStarted!: () => void;
    const oldStarted = new Promise<void>((resolve) => {
      oldRequestStarted = resolve;
    });
    const fetchMock = vi.fn<FetchLike>(() => {
      if (fetchMock.mock.calls.length === 1) {
        oldRequestStarted();
        return new Promise((resolve) => {
          finishOldRequest = resolve;
        });
      }
      return Promise.resolve(jsonResponse(userResponse("new-session-user", true)));
    });
    setTokenProvider(() => "old-session-token");
    const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });
    const oldLoad = store.ensureLoaded();
    await oldStarted;

    const logoutStorage = new Map<string, string>();
    const logoutClient = new OidcClient({
      issuer: "https://id.example",
      clientId: "zoo",
      environment: {
        origin: "https://app.example",
        href: "https://app.example/",
        assign: () => {},
        replaceUrl: () => {},
        storage: {
          getItem: (key) => logoutStorage.get(key) ?? null,
          setItem: (key, value) => void logoutStorage.set(key, value),
          removeItem: (key) => void logoutStorage.delete(key),
        },
        fetch: async () =>
          jsonResponse({
            issuer: "https://id.example",
            authorization_endpoint: "https://id.example/authorize",
            token_endpoint: "https://id.example/token",
          }),
        now: () => Date.now(),
      },
    });
    await logoutClient.logout();
    setTokenProvider(() => "new-session-token");
    await store.ensureLoaded();
    expect(store.getState()).toEqual({
      currentUserId: "new-session-user",
      isLoading: false,
      isAdmin: true,
      error: null,
    });

    finishOldRequest(jsonResponse(userResponse("old-session-user")));
    await oldLoad;
    expect(store.getState().currentUserId).toBe("new-session-user");
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("retries on the next ensureLoaded after a transient failure", async () => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi
      .fn<FetchLike>()
      .mockResolvedValueOnce(jsonResponse({ error: "boom" }, 500))
      .mockResolvedValueOnce(jsonResponse(userResponse("user-1")));
    const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });

    await store.ensureLoaded();
    expect(store.getState().currentUserId).toBeNull();
    expect(store.getState().error).toMatch(/500/);

    await store.ensureLoaded();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(store.getState()).toEqual({
      currentUserId: "user-1",
      isLoading: false,
      isAdmin: false,
      error: null,
    });
  });

  it("does not retry after a 401 (AuthGate re-login owns that path)", async () => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse({ error: "unauthorized" }, 401));
    const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });

    await store.ensureLoaded();
    await store.ensureLoaded();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(store.getState()).toEqual({
      currentUserId: null,
      isLoading: false,
      isAdmin: false,
      error: null,
    });
  });
});

it("ends loading and permits retry after a stalled current-user request times out", async () => {
  const controller = new AbortController();
  const timeout = vi.spyOn(AbortSignal, "timeout").mockReturnValue(controller.signal);
  let started!: () => void;
  const fetching = new Promise<void>((resolve) => {
    started = resolve;
  });
  const fetchMock = vi.fn<FetchLike>(
    (_, init) =>
      new Promise((_, reject) => {
        init!.signal!.addEventListener("abort", () => reject(init!.signal!.reason), { once: true });
        started();
      }),
  );
  try {
    const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });
    const loading = store.ensureLoaded();
    await fetching;
    expect(timeout).toHaveBeenCalledWith(8_000);
    controller.abort(new DOMException("Request timed out", "TimeoutError"));
    await loading;
    expect(store.getState().isLoading).toBe(false);
    expect(store.getState().error).toContain("timed out");
    fetchMock.mockResolvedValueOnce(jsonResponse(userResponse("user-1")));
    await store.ensureLoaded();
    expect(store.getState().currentUserId).toBe("user-1");
  } finally {
    timeout.mockRestore();
  }
});

it("refreshes administrator status without dropping the current identity and deduplicates refreshes", async () => {
  setTokenProvider(() => "token-123");
  const fetchMock = vi
    .fn<FetchLike>()
    .mockResolvedValueOnce(jsonResponse(userResponse("user-1", true)))
    .mockResolvedValueOnce(jsonResponse(userResponse("user-1", false)));
  const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });
  await store.ensureLoaded();
  const first = store.refresh();
  const second = store.refresh();
  expect(store.getState().currentUserId).toBe("user-1");
  await Promise.all([first, second]);
  expect(fetchMock).toHaveBeenCalledTimes(2);
  expect(store.getState().isAdmin).toBe(false);
});

it.each(["network", "server"])(
  "keeps the resolved identity after a background %s failure",
  async (failure) => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi
      .fn<FetchLike>()
      .mockResolvedValueOnce(jsonResponse(userResponse("user-1", true)))
      .mockImplementationOnce(async () => {
        if (failure === "network") throw new TypeError("Network unavailable");
        return jsonResponse({ error: "Temporary outage" }, 503);
      });
    const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });
    await store.ensureLoaded();
    await store.refresh();
    expect(store.getState()).toMatchObject({
      currentUserId: "user-1",
      isAdmin: false,
      isLoading: false,
    });
    expect(store.getState().error).toContain(failure === "network" ? "Network unavailable" : "503");
  },
);

it("clears the resolved identity when a background refresh is unauthorized", async () => {
  setTokenProvider(() => "token-123");
  const fetchMock = vi
    .fn<FetchLike>()
    .mockResolvedValueOnce(jsonResponse(userResponse("user-1", true)))
    .mockResolvedValueOnce(jsonResponse({ error: "unauthorized" }, 401));
  const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });
  await store.ensureLoaded();
  await store.refresh();
  expect(store.getState()).toMatchObject({ currentUserId: null, isAdmin: false });
});
