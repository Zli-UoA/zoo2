import { expect, it, vi } from "vite-plus/test";
import {
  AuthWorkerState,
  AUTH_WORKER_IDLE_MS,
  type AuthWorkerSession,
  type AuthWorkerEnvironment,
} from "./auth-worker-state";
import type { RefreshRequest } from "./auth-worker-protocol";

function harness() {
  const records = new Map<string, AuthWorkerSession>();
  let now = 1_000_000;
  const queues = new Map<string, Promise<unknown>>();
  const fetch = vi.fn(
    async (_input: string, _init: RequestInit) =>
      new Response(
        JSON.stringify({
          access_token: `access-${fetch.mock.calls.length}`,
          refresh_token: `refresh-${fetch.mock.calls.length}`,
          id_token: `e30.${btoa(JSON.stringify({ iss: "https://id.example", aud: "zoo", exp: 9999999999 }))}.signature`,
          expires_in: 300,
        }),
      ),
  );
  const environment: AuthWorkerEnvironment = {
    store: {
      staleSessionIds: async (cutoff) =>
        [...records]
          .filter(([, row]) => row.state === "ready" && (row.lastUsedAt ?? 0) <= cutoff)
          .map(([id]) => id),
      get: async (id) => records.get(id),
      put: async (id, record) => {
        records.set(id, record);
      },
    },
    exclusive: (id, action) => {
      const result = (queues.get(id) ?? Promise.resolve()).catch(() => {}).then(action);
      queues.set(id, result);
      return result;
    },
    fetch,
    now: () => now,
  };
  const request: RefreshRequest = {
    kind: "refresh",
    sessionId: "session",
    issuer: "https://id.example",
    tokenEndpoint: "https://id.example/token",
    clientId: "zoo",
    resource: "https://api.example",
    refreshToken: "initial",
  };
  return {
    records,
    environment,
    request,
    fetch,
    state: new AuthWorkerState(environment),
    advance: (ms: number) => {
      now += ms;
    },
  };
}

it("shares one refresh and its result across concurrent tabs", async () => {
  const h = harness();
  const results = await Promise.all(Array.from({ length: 8 }, () => h.state.run(h.request)));
  expect(h.fetch).toHaveBeenCalledTimes(1);
  expect(results.every((body) => body?.access_token === "access-1")).toBe(true);
  const form = new URLSearchParams(h.fetch.mock.calls[0][1].body as string);
  expect(form.get("refresh_token")).toBe("initial");
  expect(form.get("resource")).toBe("https://api.example");
});

it("restores after worker restart and uses the latest refresh token for stale tabs", async () => {
  const h = harness();
  await h.state.run(h.request);
  h.advance(100_000);
  const restarted = new AuthWorkerState(h.environment);
  const cached = await restarted.run(h.request);
  expect(cached?.expires_in).toBe(200);
  expect(cached?.id_token).toBeUndefined();
  expect(h.fetch).toHaveBeenCalledTimes(1);
  h.advance(150_000);
  await restarted.run(h.request);
  expect(h.fetch).toHaveBeenCalledTimes(2);
  expect(new URLSearchParams(h.fetch.mock.calls[1][1].body as string).get("refresh_token")).toBe(
    "refresh-1",
  );
});

it("does not replay a refresh after its response is lost", async () => {
  const h = harness();
  h.fetch.mockRejectedValueOnce(new TypeError("network lost"));
  await expect(h.state.run(h.request)).rejects.toThrow();
  const restarted = new AuthWorkerState(h.environment);
  await expect(restarted.run(h.request)).rejects.toThrow("session_ended");
  expect(h.fetch).toHaveBeenCalledTimes(1);
});

it("does not send a token unless pending state is committed", async () => {
  const h = harness();
  h.environment.store.put = async () => {
    throw new Error("storage unavailable");
  };
  await expect(h.state.run(h.request)).rejects.toThrow("storage unavailable");
  expect(h.fetch).not.toHaveBeenCalled();
});

it("does not revive a logged-out session from a copied tab", async () => {
  const h = harness();
  await h.state.run(h.request);
  await h.state.run({ kind: "logout", sessionId: h.request.sessionId });
  await expect(new AuthWorkerState(h.environment).run(h.request)).rejects.toThrow("session_ended");
  expect(h.fetch).toHaveBeenCalledTimes(1);
  expect(h.records.get(h.request.sessionId)?.body).toBeUndefined();
});

it("isolates accounts and rejects changed client/resource bindings", async () => {
  const h = harness();
  await h.state.run(h.request);
  await expect(h.state.run({ ...h.request, clientId: "other" })).rejects.toThrow(
    "session_mismatch",
  );
  await expect(h.state.run({ ...h.request, resource: "https://other.example" })).rejects.toThrow(
    "session_mismatch",
  );
  await h.state.run({ ...h.request, sessionId: "second-account", refreshToken: "second" });
  expect(h.fetch).toHaveBeenCalledTimes(2);
});

it.each([400, 429, 503])("does not replay a rejected refresh (%s)", async (status) => {
  const h = harness();
  h.fetch.mockResolvedValueOnce(
    new Response(JSON.stringify({ error: "invalid_grant" }), { status }),
  );
  await expect(h.state.run(h.request)).rejects.toThrow("invalid_grant");
  await expect(h.state.run(h.request)).rejects.toThrow("session_ended");
  expect(h.fetch).toHaveBeenCalledTimes(1);
});

it("rejects an invalid ID token before exposing a shared cached result", async () => {
  const h = harness();
  h.fetch.mockResolvedValueOnce(
    new Response(
      JSON.stringify({
        access_token: "wrong",
        refresh_token: "wrong",
        id_token: "invalid",
        expires_in: 300,
      }),
    ),
  );
  await expect(h.state.run(h.request)).rejects.toThrow("invalid_id_token");
  await expect(new AuthWorkerState(h.environment).run(h.request)).rejects.toThrow("session_ended");
  expect(h.fetch).toHaveBeenCalledTimes(1);
});

it("serializes an update and logout without retaining reusable credentials", async () => {
  const h = harness();
  await Promise.allSettled([
    h.state.run(h.request),
    h.state.run({ kind: "logout", sessionId: h.request.sessionId }),
  ]);
  expect(h.records.get(h.request.sessionId)?.state).toBe("ended");
  expect(h.records.get(h.request.sessionId)?.body).toBeUndefined();
});

it("defaults omitted expires_in to 300 seconds", async () => {
  const h = harness();
  h.fetch.mockResolvedValueOnce(
    new Response(JSON.stringify({ access_token: "access", refresh_token: "next" })),
  );
  const result = await h.state.run(h.request);
  expect(result?.expires_in).toBe(300);
  expect(result?.expiresAt).toBe(1_300_000);
});

it.each([0, -1, null, "300"])("rejects invalid expires_in: %s", async (expires_in) => {
  const h = harness();
  h.fetch.mockResolvedValueOnce(
    new Response(JSON.stringify({ access_token: "access", expires_in })),
  );
  await expect(h.state.run(h.request)).rejects.toThrow("invalid_token_response");
});

it("clears idle credentials on the next worker start, keeping a tombstone", async () => {
  const h = harness();
  await h.state.run(h.request);
  h.advance(AUTH_WORKER_IDLE_MS + 1);
  await expect(new AuthWorkerState(h.environment).run(h.request)).rejects.toThrow("session_ended");
  expect(h.records.get(h.request.sessionId)?.body).toBeUndefined();
  expect(h.fetch).toHaveBeenCalledTimes(1);
});

it("does not erase a session refreshed while cleanup was waiting for its lock", async () => {
  const h = harness();
  await h.state.run(h.request);
  h.advance(AUTH_WORKER_IDLE_MS + 1);
  const stale = h.environment.store.staleSessionIds.bind(h.environment.store);
  h.environment.store.staleSessionIds = async (cutoff) => {
    const ids = await stale(cutoff);
    h.records.get(h.request.sessionId)!.lastUsedAt = h.environment.now();
    return ids;
  };
  await new AuthWorkerState(h.environment).run(h.request);
  expect(h.records.get(h.request.sessionId)?.state).toBe("ready");
  expect(h.fetch).toHaveBeenCalledTimes(2);
});

it.each(["scan", "write"])(
  "keeps refreshing when unrelated cleanup fails during %s, then retries after backoff",
  async (failure) => {
    const h = harness();
    const old: AuthWorkerSession = {
      state: "ready",
      binding: "old",
      lastUsedAt: -AUTH_WORKER_IDLE_MS,
      body: { access_token: "old", refresh_token: "old", expires_in: 300 },
    };
    h.records.set("old-session", old);
    const scan = h.environment.store.staleSessionIds.bind(h.environment.store);
    const write = h.environment.store.put.bind(h.environment.store);
    let blocked = true;
    const scanner = vi.fn(async (cutoff: number) => {
      if (blocked && failure === "scan") throw new Error("scan unavailable");
      return scan(cutoff);
    });
    h.environment.store.staleSessionIds = scanner;
    h.environment.store.put = async (id, row) => {
      if (blocked && failure === "write" && id === "old-session")
        throw new Error("old row unavailable");
      await write(id, row);
    };
    await expect(h.state.run(h.request)).resolves.toMatchObject({ access_token: "access-1" });
    await expect(h.state.run(h.request)).resolves.toMatchObject({ access_token: "access-1" });
    expect(scanner).toHaveBeenCalledTimes(1);
    expect(h.records.get("old-session")?.state).toBe("ready");
    blocked = false;
    h.advance(60 * 60 * 1000);
    await expect(h.state.run(h.request)).resolves.toMatchObject({ access_token: "access-2" });
    expect(scanner).toHaveBeenCalledTimes(2);
    expect(h.records.get("old-session")?.state).toBe("ended");
  },
);

it.each([0, 1])(
  "enforces the requesting session's idle limit despite a failed scan (boundary +%s ms)",
  async (afterBoundary) => {
    const h = harness();
    await h.state.run(h.request);
    h.advance(AUTH_WORKER_IDLE_MS + afterBoundary);
    h.environment.store.staleSessionIds = async () => {
      throw new Error("scan unavailable");
    };
    await expect(h.state.run(h.request)).rejects.toThrow("session_ended");
    expect(h.records.get(h.request.sessionId)?.state).toBe("ended");
    expect(h.records.get(h.request.sessionId)?.body).toBeUndefined();
    expect(h.fetch).toHaveBeenCalledTimes(1);
  },
);

it("enforces the requesting session's idle limit while cleanup is in backoff", async () => {
  const h = harness();
  const scanner = vi.fn(async () => {
    throw new Error("scan unavailable");
  });
  h.environment.store.staleSessionIds = scanner;
  await h.state.run(h.request);
  h.records.set("idle-session", {
    ...h.records.get(h.request.sessionId)!,
    lastUsedAt: h.environment.now() - AUTH_WORKER_IDLE_MS,
  });
  await expect(h.state.run({ ...h.request, sessionId: "idle-session" })).rejects.toThrow(
    "session_ended",
  );
  expect(scanner).toHaveBeenCalledTimes(1);
  expect(h.records.get("idle-session")?.body).toBeUndefined();
  expect(h.fetch).toHaveBeenCalledTimes(1);
});

it("does not send an idle session's token if persisting its end state fails", async () => {
  const h = harness();
  await h.state.run(h.request);
  h.advance(AUTH_WORKER_IDLE_MS);
  h.environment.store.staleSessionIds = async () => {
    throw new Error("scan unavailable");
  };
  h.environment.store.put = async () => {
    throw new Error("write unavailable");
  };
  await expect(h.state.run(h.request)).rejects.toThrow("write unavailable");
  expect(h.fetch).toHaveBeenCalledTimes(1);
});
