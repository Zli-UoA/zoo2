import type {
  CryptoFailure,
  CryptoOperation,
  CryptoRequest,
  CryptoResponse,
  CryptoResults,
} from "./protocol";

/** Worker の中の操作が失敗した。`code` は `ZooCryptoError` の `code`。 */
export class CryptoWorkerError extends Error {
  readonly code: string | undefined;

  constructor(failure: CryptoFailure) {
    super(failure.message);
    this.name = failure.name;
    this.code = failure.code;
  }
}

export interface CryptoWorker {
  request<Op extends CryptoOperation["op"]>(
    operation: Extract<CryptoOperation, { op: Op }>,
  ): Promise<CryptoResults[Op]>;
  /** Worker を止め、待っている要求をすべて失敗させる。 */
  terminate(): void;
}

/**
 * zoo-crypto（wasm）を動かす Worker を起動する。
 *
 * 鍵の生成や復号はメインスレッドを止めないよう Worker で行う。暗号ストアと 1 タブだけで開く
 * 仕組み（Web Locks）は #359 で足す。
 */
export function startCryptoWorker(): CryptoWorker {
  const worker = new Worker(new URL("./crypto.worker.ts", import.meta.url), {
    type: "module",
    name: "zoo-crypto",
  });
  const pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: Error) => void }
  >();
  let sequence = 0;

  const failAll = (error: Error) => {
    for (const { reject } of pending.values()) reject(error);
    pending.clear();
  };

  worker.onmessage = ({ data }: MessageEvent<CryptoResponse>) => {
    const entry = pending.get(data.id);
    if (!entry) return;
    pending.delete(data.id);
    if (data.ok) entry.resolve(data.result);
    else entry.reject(new CryptoWorkerError(data.error));
  };
  worker.onerror = (event) => {
    event.preventDefault();
    failAll(new Error(event.message || "zoo-crypto Worker failed"));
  };

  return {
    request(operation) {
      const id = ++sequence;
      const message: CryptoRequest = { ...operation, id };
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
        worker.postMessage(message);
      });
    },
    terminate() {
      worker.terminate();
      failAll(new Error("zoo-crypto Worker terminated"));
    },
  };
}
