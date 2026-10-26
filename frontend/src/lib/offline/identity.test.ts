import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import {
  OFFLINE_LAST_USER_KEY,
  clearPendingOfflineDeletion,
  forgetOfflineUserId,
  markOfflineDeletionPending,
  offlineDatabaseName,
  readLastOfflineUserId,
  readPendingOfflineDeletion,
  rememberOfflineUserId,
  resolveOfflineNamespace,
} from "./identity";

function stubStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
  vi.stubGlobal("window", { localStorage: storage });
  return values;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("resolveOfflineNamespace", () => {
  it("prefers a fixed dev user over an unresolved OIDC user", () => {
    expect(resolveOfflineNamespace({ devUserId: "dev-1", currentUserId: null })).toBe("dev-dev-1");
  });

  it("uses the resolved OIDC user, then the last used user, then anonymous", () => {
    stubStorage();
    expect(resolveOfflineNamespace({ devUserId: null, currentUserId: "alice" })).toBe("user-alice");

    rememberOfflineUserId("bob");
    expect(readLastOfflineUserId()).toBe("bob");
    expect(resolveOfflineNamespace({ devUserId: null, currentUserId: null })).toBe("user-bob");
  });

  it("falls back to anonymous when storage is empty or unavailable", () => {
    stubStorage();
    expect(resolveOfflineNamespace({ devUserId: null, currentUserId: null })).toBe("anonymous");

    vi.stubGlobal("window", {
      get localStorage() {
        throw new Error("blocked");
      },
    });
    expect(readLastOfflineUserId()).toBeNull();
  });
});

describe("offlineDatabaseName", () => {
  it("separates users into different OPFS databases", () => {
    expect(offlineDatabaseName("user-alice")).not.toBe(offlineDatabaseName("user-bob"));
    expect(offlineDatabaseName("user-alice")).toMatch(/^zoo-offline-[a-z0-9]+\.sqlite$/);
  });

  it("hashes long or unsafe namespaces into a short filename", () => {
    const name = offlineDatabaseName(`user/${"a".repeat(200)}:b c`);
    expect(name).toMatch(/^zoo-offline-[a-z0-9]+\.sqlite$/);
    expect(name.length).toBeLessThanOrEqual(40);
  });
});

describe("pending offline deletion", () => {
  it("remembers a failed logout deletion until the next startup", () => {
    stubStorage();
    expect(readPendingOfflineDeletion()).toBeNull();
    markOfflineDeletionPending("user-alice");
    expect(readPendingOfflineDeletion()).toBe("user-alice");
    clearPendingOfflineDeletion();
    expect(readPendingOfflineDeletion()).toBeNull();
  });
});

describe("rememberOfflineUserId", () => {
  it("clears only the stored user", () => {
    const values = stubStorage({ [OFFLINE_LAST_USER_KEY]: "alice", other: "keep" });
    forgetOfflineUserId("bob");
    expect(values.get(OFFLINE_LAST_USER_KEY)).toBe("alice");
    forgetOfflineUserId("alice");
    expect(values.has(OFFLINE_LAST_USER_KEY)).toBe(false);
    expect(values.get("other")).toBe("keep");
  });
});
