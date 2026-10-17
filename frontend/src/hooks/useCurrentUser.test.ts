import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import type { FetchLike } from "../lib/api/client";
import { resetTokenProvider, setTokenProvider, setUnauthorizedHandler } from "../lib/auth";
import { resolveConfig } from "../lib/config";
import {
  createCurrentUserStore,
  resolveCurrentUser,
  type CurrentUserState,
} from "./useCurrentUser";

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

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function userResponse(id: string): Record<string, unknown> {
  return {
    id,
    user_name: "smoke",
    display_name: "Smoke User",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: null,
  };
}

afterEach(() => {
  resetTokenProvider();
  setUnauthorizedHandler(null);
});

describe("resolveCurrentUser", () => {
  it("uses VITE_DEV_USER_ID without calling the API when OIDC is unset", async () => {
    const fetchMock = vi.fn();
    const resolved = await resolveCurrentUser(DEV_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: "dev-user-1", error: null });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("resolves the id from GET /api/users/me with a Bearer token when OIDC is configured", async () => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse(userResponse("user-1")));
    const resolved = await resolveCurrentUser(OIDC_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: "user-1", error: null });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("http://localhost:8000/api/users/me");
    expect(new Headers(init?.headers).get("Authorization")).toBe("Bearer token-123");
  });

  it("returns null and leaves re-login to the unauthorized handler on 401", async () => {
    const onUnauthorized = vi.fn();
    setUnauthorizedHandler(onUnauthorized);
    const fetchMock = vi.fn(async () => jsonResponse({ error: "unauthorized" }, 401));
    const resolved = await resolveCurrentUser(OIDC_CONFIG, { fetch: fetchMock });
    expect(resolved).toEqual({ currentUserId: null, error: null });
    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });

  it("reports other failures without a current user", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ error: "boom" }, 500));
    const resolved = await resolveCurrentUser(OIDC_CONFIG, { fetch: fetchMock });
    expect(resolved.currentUserId).toBeNull();
    expect(resolved.error).toMatch(/500/);
  });
});

describe("createCurrentUserStore", () => {
  it("is already resolved from VITE_DEV_USER_ID in dev mode", () => {
    const store = createCurrentUserStore(DEV_CONFIG);
    expect(store.getState()).toEqual({
      currentUserId: "dev-user-1",
      isLoading: false,
      error: null,
    });
  });

  it("resolves once and notifies subscribers", async () => {
    setTokenProvider(() => "token-123");
    const fetchMock = vi.fn<FetchLike>(async () => jsonResponse(userResponse("user-1")));
    const store = createCurrentUserStore(OIDC_CONFIG, { fetch: fetchMock });
    const states: CurrentUserState[] = [];
    store.subscribe(() => states.push(store.getState()));
    expect(store.getState()).toEqual({ currentUserId: null, isLoading: true, error: null });

    await store.ensureLoaded();
    await store.ensureLoaded();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(store.getState()).toEqual({ currentUserId: "user-1", isLoading: false, error: null });
    expect(states.at(-1)?.currentUserId).toBe("user-1");
  });
});
