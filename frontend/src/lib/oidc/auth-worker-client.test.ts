import { expect, it, vi } from "vite-plus/test";
import type { RefreshRequest } from "./auth-worker-protocol";
import { requestAuthWorker, supportsAuthWorker } from "./auth-worker-client";

it("rejects all pending calls immediately when the worker script fails to load", async () => {
  const instances: FakeWorker[] = [];
  class FakeWorker extends EventTarget {
    readonly port = new MessageChannel().port1;
    constructor() {
      super();
      instances.push(this);
    }
  }
  vi.stubGlobal("SharedWorker", FakeWorker);
  const request: RefreshRequest = {
    kind: "refresh",
    sessionId: "session",
    issuer: "https://id.example",
    tokenEndpoint: "https://id.example/token",
    clientId: "zoo",
    resource: null,
    refreshToken: "test",
  };
  try {
    const first = requestAuthWorker(request);
    const second = requestAuthWorker(request);
    const assertions = Promise.all([
      expect(first).rejects.toThrow("auth_worker_load_failed"),
      expect(second).rejects.toThrow("auth_worker_load_failed"),
    ]);
    expect(instances).toHaveLength(1);
    instances[0].dispatchEvent(new Event("error", { cancelable: true }));
    await assertions;
  } finally {
    vi.unstubAllGlobals();
  }
});

it.each(["SharedWorker", "indexedDB", "locks"])("does not enable refresh without %s", (missing) => {
  vi.stubGlobal("SharedWorker", class {});
  vi.stubGlobal("indexedDB", { open() {} });
  vi.stubGlobal("navigator", { locks: { request() {} } });
  try {
    expect(supportsAuthWorker()).toBe(true);
    if (missing === "locks") vi.stubGlobal("navigator", {});
    else vi.stubGlobal(missing, undefined);
    expect(supportsAuthWorker()).toBe(false);
  } finally {
    vi.unstubAllGlobals();
  }
});
