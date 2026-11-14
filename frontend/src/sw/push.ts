/**
 * Service Worker（`public/sw.js`）が `importScripts("/sw-push.js")` で読む、暗号化チャンネルの
 * Push の復号（#365。docs/e2ee.md の「Web Push の復号」）。
 *
 * `vite.sw-push.config.ts` が、このファイルを 1 つの classic script（IIFE）の `sw-push.js` にする。
 * Service Worker では `import()` を使えず、`importScripts` は起動時と install のあいだしか読めない
 * ので、固定の名前で置く。wasm は Push を受けたときに初めて読む。
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
import { decryptPush } from "../lib/e2ee/push/pushDecrypt";
import { openSnapshotBackend, readCryptoDatabaseFiles } from "../lib/e2ee/push/snapshotBackend";

// tsconfig の lib は DOM なので、Service Worker のグローバルは使う分だけ型を付ける。
interface WindowClient {
  postMessage(message: unknown, transfer: Transferable[]): void;
}
const scope = self as unknown as {
  clients: {
    matchAll(options: { type: "window"; includeUncontrolled: boolean }): Promise<WindowClient[]>;
  };
  zooDecryptPush?: (e2ee: unknown) => Promise<string | null>;
};

/** Push の処理に掛けてよい時間。過ぎたら「新しいメッセージ」のまま出す。 */
const DECRYPT_TIMEOUT_MS = 10_000;

let cryptoReady: Promise<unknown> | null = null;
let sqliteModule: Promise<unknown> | null = null;

/** wasm は Push を受けたときに読む（fetch のたびに Service Worker を起こすので、起動時は読まない）。 */
function loadCrypto(): Promise<unknown> {
  cryptoReady ??= init({ module_or_path: cryptoWasmUrl }).catch((error: unknown) => {
    cryptoReady = null;
    throw error;
  });
  return cryptoReady;
}

function loadSqlite(): Promise<unknown> {
  sqliteModule ??= SQLiteESMFactory({ locateFile: () => sqliteWasmUrl }).catch((error: unknown) => {
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
    openSnapshot: async (name) => {
      const files = await readCryptoDatabaseFiles(name);
      if (!files) return null;
      await loadCrypto();
      return openSnapshotBackend(await loadSqlite(), files);
    },
    askTabs,
  });
}

scope.zooDecryptPush = (e2ee) => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<null>((resolve) => {
    timer = setTimeout(() => resolve(null), DECRYPT_TIMEOUT_MS);
  });
  return Promise.race([decrypt(e2ee).catch(() => null), timeout]).finally(() =>
    clearTimeout(timer),
  );
};
