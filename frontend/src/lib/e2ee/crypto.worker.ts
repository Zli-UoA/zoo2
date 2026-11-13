import init from "@zoo/crypto-wasm";

import { CryptoOperations, toCryptoFailure } from "./operations";
import type { CryptoRequest, CryptoResponse } from "./protocol";
import { openSqliteCryptoBackend, removeCryptoDatabaseFiles } from "./store/sqliteBackend";

// tsconfig の lib は DOM なので、Worker のグローバルは使う分だけ型を付ける。
const scope = self as unknown as {
  onmessage: ((event: MessageEvent<CryptoRequest>) => void) | null;
  postMessage(message: CryptoResponse): void;
};

// wasm（*_bg.wasm）はビルドで assets/ に出力され、Service Worker が他の assets と同じく保存する。
// 実行には CSP の script-src に 'wasm-unsafe-eval' が要る（infra/frontend-nginx.conf）。
const ready = init();

const operations = new CryptoOperations({
  open: openSqliteCryptoBackend,
  remove: removeCryptoDatabaseFiles,
});

// 暗号ストアの状態を書き換える操作は、届いた順に 1 つずつ行う。
let queue: Promise<unknown> = Promise.resolve();

scope.onmessage = ({ data }) => {
  queue = queue.then(async () => {
    try {
      await ready;
      scope.postMessage({ id: data.id, ok: true, result: await operations.run(data) });
    } catch (error) {
      scope.postMessage({ id: data.id, ok: false, error: toCryptoFailure(error) });
    }
  });
};
