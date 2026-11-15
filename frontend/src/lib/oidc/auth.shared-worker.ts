import type { AuthWorkerRequest, AuthWorkerResponse } from "./auth-worker-protocol";
import { AuthWorkerError, AuthWorkerState } from "./auth-worker-state";
import { authWorkerStore } from "./auth-worker-store";

// ページの型定義と共存するため、SharedWorkerのグローバルは使用分だけ宣言する。
const scope = self as unknown as { onconnect: ((event: MessageEvent) => void) | null };
const state = new AuthWorkerState({
  store: authWorkerStore,
  // 配備をまたぐ旧・新ワーカーも同じロックと保存先を使う。
  exclusive: (id, action) =>
    navigator.locks.request(`zoo-oidc:${id}`, { signal: AbortSignal.timeout(15_000) }, action),
  fetch: (input, init) => fetch(input, init),
  now: () => Date.now(),
});

scope.onconnect = ({ ports: [port] }) => {
  port.onmessage = async (event: MessageEvent<AuthWorkerRequest>) => {
    const reply = event.ports[0];
    if (!reply) return;
    let response: AuthWorkerResponse;
    try {
      response = { ok: true, body: await state.run(event.data) };
    } catch (error) {
      response = {
        ok: false,
        code: error instanceof AuthWorkerError ? error.code : "auth_worker_failed",
        ...(error instanceof AuthWorkerError ? { status: error.status } : {}),
      };
    }
    reply.postMessage(response);
    reply.close();
  };
  port.start();
};
