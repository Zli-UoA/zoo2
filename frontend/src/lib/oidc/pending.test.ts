import { beforeEach, describe, expect, it } from "vite-plus/test";

import {
  MAX_REDIRECT_ATTEMPTS,
  PENDING_TTL_MS,
  REDIRECT_ATTEMPT_WINDOW_MS,
  clearAuthorizeRedirectPending,
  clearLoggedOut,
  clearPending,
  clearRedirectAttempts,
  isAuthorizeRedirectPending,
  isLoggedOut,
  markAuthorizeRedirectPending,
  markLoggedOut,
  markRedirectStarted,
  readLastRedirectAt,
  readUnauthorizedRedirectState,
  recordRedirectAttempt,
  savePending,
  takePending,
  type AuthStorage,
  type PendingAuthorization,
} from "./pending";

function memoryStorage(): AuthStorage {
  const map = new Map<string, string>();
  return {
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => void map.set(key, value),
    removeItem: (key) => void map.delete(key),
  };
}

function pending(overrides: Partial<PendingAuthorization> = {}): PendingAuthorization {
  return {
    state: "state-1",
    nonce: "nonce-1",
    verifier: "verifier-1",
    returnTo: "/channels/abc",
    createdAt: 1_000,
    ...overrides,
  };
}

let storage: AuthStorage;

beforeEach(() => {
  storage = memoryStorage();
});

describe("pending authorization", () => {
  it("round-trips and is single-use", () => {
    savePending(storage, pending());
    expect(takePending(storage, 2_000)).toEqual(pending());
    expect(takePending(storage, 2_000)).toBeNull();
  });

  it("rejects expired pending requests", () => {
    savePending(storage, pending());
    expect(takePending(storage, 1_000 + PENDING_TTL_MS + 1)).toBeNull();
  });

  it("rejects malformed JSON and wrong shapes", () => {
    storage.setItem("zoo.oidc.pending", "{not json");
    expect(takePending(storage)).toBeNull();
    storage.setItem("zoo.oidc.pending", JSON.stringify({ state: "only-state" }));
    expect(takePending(storage)).toBeNull();
  });

  it("clearPending removes the stored request", () => {
    savePending(storage, pending());
    clearPending(storage);
    expect(takePending(storage, 2_000)).toBeNull();
  });
});

describe("logged out flag", () => {
  it("toggles", () => {
    expect(isLoggedOut(storage)).toBe(false);
    markLoggedOut(storage);
    expect(isLoggedOut(storage)).toBe(true);
    clearLoggedOut(storage);
    expect(isLoggedOut(storage)).toBe(false);
  });
});

describe("redirect attempts", () => {
  it("counts attempts within the window", () => {
    expect(recordRedirectAttempt(storage, 1_000)).toBe(1);
    expect(recordRedirectAttempt(storage, 2_000)).toBe(2);
    expect(recordRedirectAttempt(storage, 3_000)).toBe(3);
  });

  it("prunes attempts older than the window", () => {
    recordRedirectAttempt(storage, 1_000);
    recordRedirectAttempt(storage, 2_000);
    expect(recordRedirectAttempt(storage, 2_000 + REDIRECT_ATTEMPT_WINDOW_MS + 1)).toBe(1);
  });

  it("can be cleared after a successful authenticated request", () => {
    for (let index = 0; index <= MAX_REDIRECT_ATTEMPTS; index += 1) {
      recordRedirectAttempt(storage, 1_000 + index);
    }
    clearRedirectAttempts(storage);
    expect(recordRedirectAttempt(storage, 2_000)).toBe(1);
  });

  it("recovers from malformed state", () => {
    storage.setItem("zoo.oidc.redirectAttempts", "{not json");
    expect(recordRedirectAttempt(storage, 1_000)).toBe(1);
  });
});

describe("last redirect", () => {
  it("records and reads the redirect start time", () => {
    expect(readLastRedirectAt(storage)).toBeNull();
    markRedirectStarted(storage, 1_234);
    expect(readLastRedirectAt(storage)).toBe(1_234);
  });

  it("treats malformed values as absent", () => {
    storage.setItem("zoo.oidc.lastRedirectAt", "not-a-number");
    expect(readLastRedirectAt(storage)).toBeNull();
  });
});

describe("authorize redirect pending", () => {
  it("tracks whether an authorize redirect is in progress", () => {
    expect(isAuthorizeRedirectPending(storage, 1_000)).toBe(false);
    markAuthorizeRedirectPending(storage, 1_000);
    expect(isAuthorizeRedirectPending(storage, 1_500)).toBe(true);
    clearAuthorizeRedirectPending(storage);
    expect(isAuthorizeRedirectPending(storage, 1_500)).toBe(false);
  });

  it("drops stale and malformed values", () => {
    markAuthorizeRedirectPending(storage, 1_000);
    expect(isAuthorizeRedirectPending(storage, 1_000 + PENDING_TTL_MS + 1)).toBe(false);
    expect(storage.getItem("zoo.oidc.authorizeRedirectAt")).toBeNull();

    storage.setItem("zoo.oidc.authorizeRedirectAt", "not-a-number");
    expect(isAuthorizeRedirectPending(storage, 1_000)).toBe(false);
  });

  it("ignores 401s while redirecting and treats a just-finished redirect as a loop", () => {
    markRedirectStarted(storage, 1_000);
    markAuthorizeRedirectPending(storage, 1_000);
    expect(readUnauthorizedRedirectState(storage, 2_000, 10_000)).toBe("redirecting");

    clearAuthorizeRedirectPending(storage);
    expect(readUnauthorizedRedirectState(storage, 2_000, 10_000)).toBe("recent");
    expect(readUnauthorizedRedirectState(storage, 12_000, 10_000)).toBe("none");
  });

  it("stops ignoring 401s when a redirect marker is stale", () => {
    markRedirectStarted(storage, 1_000);
    markAuthorizeRedirectPending(storage, 1_000);

    expect(readUnauthorizedRedirectState(storage, 11_001, 10_000)).toBe("none");
    expect(storage.getItem("zoo.oidc.authorizeRedirectAt")).toBeNull();
  });
});
