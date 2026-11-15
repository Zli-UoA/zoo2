import { expect, it, vi } from "vite-plus/test";
import {
  AuthWorkerState,
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
  await Promise.all([
    h.state.run(h.request),
    h.state.run({ kind: "logout", sessionId: h.request.sessionId }),
  ]);
  expect(h.records.get(h.request.sessionId)?.state).toBe("ended");
  expect(h.records.get(h.request.sessionId)?.body).toBeUndefined();
});
