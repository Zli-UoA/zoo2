import { authWorkerStore } from "./auth-worker-store";
import type {
  AuthTokenResponse,
  AuthWorkerRequest,
  AuthWorkerResponse,
} from "./auth-worker-protocol";

let worker: SharedWorker | null = null;

export async function requestAuthWorker(
  request: AuthWorkerRequest,
): Promise<AuthTokenResponse | null> {
  try {
    if (!worker)
      worker = new SharedWorker(new URL("./auth.shared-worker.ts", import.meta.url), {
        type: "module",
        name: `zoo-oidc:${import.meta.url}`,
      });
    return await callAuthWorker(worker.port, request);
  } catch (error) {
    worker?.port.close();
    worker = null;
    if (request.kind !== "logout") throw error;
    // Workerを起動できない環境でもログアウトは可能にする。更新の代行はしない。
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
): Promise<AuthTokenResponse | null> {
  return new Promise((resolve, reject) => {
    const channel = new MessageChannel();
    const finish = () => {
      clearTimeout(timeout);
      channel.port1.close();
      channel.port2.close();
    };
    const timeout = setTimeout(() => {
      finish();
      reject(new Error("auth_worker_timeout"));
    }, 30_000);
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
