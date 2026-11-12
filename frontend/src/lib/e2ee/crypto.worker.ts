import init from "@zoo/crypto-wasm";

import { runCryptoOperation, toCryptoFailure } from "./operations";
import type { CryptoRequest, CryptoResponse } from "./protocol";

// tsconfig の lib は DOM なので、Worker のグローバルは使う分だけ型を付ける。
const scope = self as unknown as {
  onmessage: ((event: MessageEvent<CryptoRequest>) => void) | null;
  postMessage(message: CryptoResponse): void;
};

// wasm（*_bg.wasm）はビルドで assets/ に出力され、Service Worker が他の assets と同じく保存する。
// 実行には CSP の script-src に 'wasm-unsafe-eval' が要る（infra/frontend-nginx.conf）。
const ready = init();

scope.onmessage = async ({ data }) => {
  try {
    await ready;
    scope.postMessage({ id: data.id, ok: true, result: runCryptoOperation(data) });
  } catch (error) {
    scope.postMessage({ id: data.id, ok: false, error: toCryptoFailure(error) });
  }
};
