/**
 * Service Worker（`public/sw.js`）が `importScripts("/sw-push.js")` で読む、暗号化チャンネルの
 * Push の復号（#365。docs/e2ee.md の「Web Push の復号」）。
 *
 * `vite.sw-push.config.ts` が、このファイルを 1 つの classic script（IIFE）の `sw-push.js` にする。
 * Service Worker では `import()` を使えず、`importScripts` は起動時と install のあいだしか読めない
 * ので、固定の名前で置く。wasm は install で専用のキャッシュ（{@link WASM_CACHE}）に保存し、
 * Push を受けたときに初めてコンパイルする。
 */
import SQLiteESMFactory from "@journeyapps/wa-sqlite/dist/wa-sqlite.mjs";
import sqliteWasmUrl from "@journeyapps/wa-sqlite/dist/wa-sqlite.wasm?url";
import init from "@zoo/crypto-wasm";
import cryptoWasmUrl from "@zoo/crypto-wasm/zoo_crypto_wasm_bg.wasm?url";

import {
  PUSH_TAB_TIMEOUT_MS,
  type PushDecryptReply,
  type PushDecryptRequest,
} from "../lib/e2ee/push/protocol";
import { decryptPush, serializeDecryption } from "../lib/e2ee/push/pushDecrypt";
import { readPushRecipient } from "../lib/e2ee/push/recipient";
import { openSnapshotBackend, readCryptoDatabaseFiles } from "../lib/e2ee/push/snapshotBackend";

// tsconfig の lib は DOM なので、Service Worker のグローバルは使う分だけ型を付ける。
interface WindowClient {
  postMessage(message: unknown, transfer: Transferable[]): void;
}
const scope = self as unknown as {
  addEventListener(
    type: "install" | "activate",
    listener: (event: { waitUntil(promise: Promise<unknown>): void }) => void,
  ): void;
  clients: {
    matchAll(options: { type: "window"; includeUncontrolled: boolean }): Promise<WindowClient[]>;
  };
  zooDecryptPush?: (e2ee: unknown) => Promise<string | null>;
};

/** Push の処理に掛けてよい時間（前の Push を待つ時間を含む）。過ぎたら「新しいメッセージ」のまま出す。 */
const DECRYPT_TIMEOUT_MS = 10_000;

/**
 * wasm を置くキャッシュ。デプロイで wasm の名前（ハッシュ）が変わっても、動いている Service Worker の
 * `sw-push.js` が指す wasm を返せるよう、shell のキャッシュとは分けて持つ。新しい Service Worker の
 * activate で、今の 2 つ以外を消す。
 */
const WASM_CACHE = "zoo-push-wasm-v1";
const WASM_URLS = [cryptoWasmUrl, sqliteWasmUrl];
/** wasm を 1 つ取るのに掛けてよい時間。 */
const WASM_FETCH_TIMEOUT_MS = 8_000;

/** wasm のバイト列。キャッシュになければ取って保存する。 */
async function wasmBytes(url: string): Promise<ArrayBuffer> {
  const cache = await caches.open(WASM_CACHE);
  const cached = await cache.match(url);
  if (cached) return cached.arrayBuffer();
  const response = await fetch(url, { signal: AbortSignal.timeout(WASM_FETCH_TIMEOUT_MS) });
  if (!response.ok) throw new Error(`wasm の取得に失敗しました: ${response.status}`);
  const bytes = await response.arrayBuffer();
  await cache
    .put(url, new Response(bytes, { headers: { "content-type": "application/wasm" } }))
    .catch(() => undefined);
  return bytes;
}

scope.addEventListener("install", (event) => {
  // 保存できなくても install は止めない（Push を受けたときに取り直す）。
  event.waitUntil(Promise.all(WASM_URLS.map(wasmBytes)).catch(() => undefined));
});

scope.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(WASM_CACHE);
      const keep = new Set(WASM_URLS.map((url) => new URL(url, self.location.href).href));
      for (const request of await cache.keys()) {
        if (!keep.has(request.url)) await cache.delete(request);
      }
    })().catch(() => undefined),
  );
});

let cryptoReady: Promise<unknown> | null = null;
let sqliteModule: Promise<unknown> | null = null;

/** wasm は Push を受けたときにコンパイルする（fetch のたびに Service Worker を起こすので、起動時はしない）。 */
function loadCrypto(): Promise<unknown> {
  cryptoReady ??= wasmBytes(cryptoWasmUrl)
    .then((bytes) => init({ module_or_path: bytes }))
    .catch((error: unknown) => {
      cryptoReady = null;
      throw error;
    });
  return cryptoReady;
}

function loadSqlite(): Promise<unknown> {
  sqliteModule ??= wasmBytes(sqliteWasmUrl)
    .then((wasmBinary) => SQLiteESMFactory({ wasmBinary }))
    .catch((error: unknown) => {
      sqliteModule = null;
      throw error;
    });
  return sqliteModule;
}

/** 暗号ストアを開いているタブに頼む。最初に復号できたタブの本文を返す。 */
async function askTabs(request: PushDecryptRequest): Promise<string | null> {
  const windows = await scope.clients.matchAll({ type: "window", includeUncontrolled: true });
  if (windows.length === 0) return null;
  return new Promise((resolve) => {
    let waiting = windows.length;
    let settled = false;
    const channels: MessageChannel[] = [];
    const finish = (body: string | null) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      for (const channel of channels) channel.port1.close();
      resolve(body);
    };
    const timer = setTimeout(() => finish(null), PUSH_TAB_TIMEOUT_MS);
    const declined = () => {
      waiting -= 1;
      if (waiting === 0) finish(null);
    };
    for (const client of windows) {
      const channel = new MessageChannel();
      channels.push(channel);
      channel.port1.onmessage = ({ data }: MessageEvent<PushDecryptReply>) => {
        if (typeof data?.body === "string") finish(data.body);
        else declined();
      };
      try {
        client.postMessage(request, [channel.port2]);
      } catch {
        declined();
      }
    }
  });
}

async function decrypt(e2ee: unknown): Promise<string | null> {
  return decryptPush(e2ee, {
    withLockIfAvailable: (name, callback) =>
      navigator.locks.request(name, { ifAvailable: true }, (lock) => callback(lock !== null)),
    readFiles: readCryptoDatabaseFiles,
    openSnapshot: async (files) => {
      const [, sqlite] = await Promise.all([loadCrypto(), loadSqlite()]);
      return openSnapshotBackend(sqlite, files);
    },
    askTabs,
    currentUser: readPushRecipient,
  });
}

// 同時に届いた Push は 1 件ずつ復号する（並べると、後の Push が前の Push の持つロックを、開いて
// いるタブのものと取り違える）。上限は、前の Push を待つ時間も含める。
scope.zooDecryptPush = serializeDecryption(decrypt, DECRYPT_TIMEOUT_MS);
