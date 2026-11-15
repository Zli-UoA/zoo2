import { expect, it } from "vite-plus/test";
import { callAuthWorker } from "./auth-worker-client";
import type { RefreshRequest } from "./auth-worker-protocol";

it("shares refresh state across ports and restores it in a new worker", async () => {
  const name = crypto.randomUUID();
  const first = new SharedWorker(new URL("./auth.shared-worker.ts", import.meta.url), {
    type: "module",
    name,
  });
  const second = new SharedWorker(new URL("./auth.shared-worker.ts", import.meta.url), {
    type: "module",
    name,
  });
  const restarted = new SharedWorker(new URL("./auth.shared-worker.ts", import.meta.url), {
    type: "module",
    name: `${name}-restart`,
  });
  const body = { access_token: "access", refresh_token: "rotated", expires_in: 300 };
  const request: RefreshRequest = {
    kind: "refresh",
    sessionId: crypto.randomUUID(),
    issuer: "https://id.example",
    tokenEndpoint: `data:application/json,${encodeURIComponent(JSON.stringify(body))}`,
    clientId: "zoo",
    resource: null,
    refreshToken: "initial",
  };
  try {
    const results = await Promise.all([
      callAuthWorker(first.port, request),
      callAuthWorker(second.port, request),
    ]);
    expect(results.map((value) => value?.refresh_token)).toEqual(["rotated", "rotated"]);
    expect((await callAuthWorker(restarted.port, request))?.access_token).toBe("access");
    await callAuthWorker(first.port, { kind: "logout", sessionId: request.sessionId });
    await expect(callAuthWorker(second.port, request)).rejects.toThrow("session_ended");
  } finally {
    first.port.close();
    second.port.close();
    restarted.port.close();
  }
});

it("can still log out when SharedWorker is unavailable", async () => {
  const { requestAuthWorker } = await import("./auth-worker-client");
  const { authWorkerStore } = await import("./auth-worker-store");
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "SharedWorker")!;
  const sessionId = crypto.randomUUID();
  try {
    Object.defineProperty(globalThis, "SharedWorker", { configurable: true, value: undefined });
    await requestAuthWorker({ kind: "logout", sessionId });
    expect((await authWorkerStore.get(sessionId))?.state).toBe("ended");
  } finally {
    Object.defineProperty(globalThis, "SharedWorker", descriptor);
  }
});

it("sweeps idle credentials from IndexedDB while retaining the end marker", async () => {
  const { authWorkerStore } = await import("./auth-worker-store");
  const { AUTH_WORKER_IDLE_MS } = await import("./auth-worker-state");
  const staleId = crypto.randomUUID();
  await authWorkerStore.put(staleId, {
    state: "ready",
    binding: "old",
    lastUsedAt: Date.now() - AUTH_WORKER_IDLE_MS - 1,
    body: { access_token: "old-access", refresh_token: "old-refresh", expires_in: 300 },
    expiresAt: 1,
  });
  const worker = new SharedWorker(new URL("./auth.shared-worker.ts", import.meta.url), {
    type: "module",
    name: crypto.randomUUID(),
  });
  try {
    await callAuthWorker(worker.port, {
      kind: "refresh",
      sessionId: crypto.randomUUID(),
      issuer: "https://id.example",
      clientId: "zoo",
      resource: null,
      refreshToken: "initial",
      tokenEndpoint: `data:application/json,${encodeURIComponent(JSON.stringify({ access_token: "new", refresh_token: "rotated", expires_in: 300 }))}`,
    });
    const saved = await authWorkerStore.get(staleId);
    expect(saved?.state).toBe("ended");
    expect(saved?.body).toBeUndefined();
  } finally {
    worker.port.close();
  }
});
