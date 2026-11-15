import { authWorkerStore } from "./auth-worker-store";
import type {
  AuthWorkerRequest,
  AuthWorkerResponse,
  AuthWorkerResult,
} from "./auth-worker-protocol";

interface WorkerConnection {
  instance: SharedWorker;
  failed: AbortController;
}
let worker: WorkerConnection | null = null;

export function supportsAuthWorker(): boolean {
  return (
    typeof SharedWorker === "function" &&
    typeof indexedDB !== "undefined" &&
    typeof indexedDB.open === "function" &&
    typeof navigator !== "undefined" &&
    typeof navigator.locks?.request === "function"
  );
}

function disconnect(connection: WorkerConnection, error: Error): void {
  connection.failed.abort(error);
  connection.instance.port.close();
  if (worker === connection) worker = null;
}

export async function requestAuthWorker(
  request: AuthWorkerRequest,
): Promise<AuthWorkerResult | null> {
  let connection: WorkerConnection | null = null;
  try {
    if (!worker) {
      const instance = new SharedWorker(new URL("./auth.shared-worker.ts", import.meta.url), {
        type: "module",
        name: `zoo-oidc:${import.meta.url}`,
      });
      const created = { instance, failed: new AbortController() };
      instance.addEventListener("error", (event) => {
        event.preventDefault();
        disconnect(created, new Error("auth_worker_load_failed"));
      });
      worker = created;
    }
    connection = worker;
    return await callAuthWorker(connection.instance.port, request, connection.failed.signal);
  } catch (error) {
    if (
      connection &&
      error instanceof Error &&
      ["auth_worker_timeout", "auth_worker_message_failed"].includes(error.message)
    ) {
      disconnect(connection, error);
    }
    if (request.kind !== "logout") throw error;
    // Workerを起動できない環境でも終了状態だけ保存する。更新は代行しない。
    const end = () => authWorkerStore.put(request.sessionId, { binding: "", state: "ended" });
    if (typeof navigator.locks?.request === "function") {
      await navigator.locks.request(
        `zoo-oidc:${request.sessionId}`,
        { signal: AbortSignal.timeout(15_000) },
        end,
      );
    } else {
      await end();
    }
    return null;
  }
}

export function callAuthWorker(
  port: MessagePort,
  request: AuthWorkerRequest,
  failed?: AbortSignal,
): Promise<AuthWorkerResult | null> {
  return new Promise((resolve, reject) => {
    if (failed?.aborted) {
      reject(failed.reason);
      return;
    }
    const channel = new MessageChannel();
    const finish = () => {
      clearTimeout(timeout);
      failed?.removeEventListener("abort", abort);
      channel.port1.close();
      channel.port2.close();
    };
    const abort = () => {
      finish();
      reject(failed?.reason);
    };
    const timeout = setTimeout(() => {
      finish();
      reject(new Error("auth_worker_timeout"));
    }, 30_000);
    failed?.addEventListener("abort", abort, { once: true });
    channel.port1.onmessage = ({ data }: MessageEvent<AuthWorkerResponse>) => {
      finish();
      if (data.ok) resolve(data.body);
      else reject(new Error(data.code));
    };
    channel.port1.onmessageerror = () => {
      finish();
      reject(new Error("auth_worker_message_failed"));
    };
    try {
      port.postMessage(request, [channel.port2]);
    } catch (error) {
      finish();
      reject(error);
    }
  });
}
