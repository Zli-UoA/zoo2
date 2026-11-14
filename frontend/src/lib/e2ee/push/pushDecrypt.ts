import { DeviceAccount, InboundSessionStore, type EncryptedMessage } from "@zoo/crypto-wasm";

import { cryptoDatabaseName } from "../identity";
import { inboundSessionKey, type CryptoBackend } from "../store/backend";
import { META_DEVICE_ID, META_USER_ID } from "../store/cryptoStore";
import {
  WRAPPED_STORAGE_KEY_META,
  openStorageKey,
  type WrappingKeyStore,
} from "../store/storageKey";
import {
  PUSH_DECRYPT_REQUEST,
  notificationBody,
  parsePushE2ee,
  pushNamespaces,
  type PushDecryptRequest,
} from "./protocol";

/**
 * Service Worker での Push の復号（`src/sw/push.ts` が使う。wasm を使うので、タブの画面からは
 * 読まない。タブ側の応答は `protocol.ts`）。
 */

/**
 * 暗号ストアの値（写し）から、1 件を復号する。復号できなければ `null`。
 *
 * 暗号ストアを開くとき（`CryptoStore.open`）と違い、何も書かない。保存鍵がなくても作り直さず、
 * 端末も作らない。受信用セッションの照合（リプレイと版の記録）はメモリーにだけ置いて捨てる。
 */
export async function decryptFromStore(options: {
  /** 暗号ストアの DB の名前（`cryptoDatabaseName`）。保存鍵を包む鍵の名前でもある。 */
  name: string;
  recipientId: string;
  backend: Pick<CryptoBackend, "read" | "readInboundSession">;
  message: EncryptedMessage;
  keys?: WrappingKeyStore;
}): Promise<string | null> {
  const snapshot = await options.backend.read();
  const deviceId = snapshot.meta[META_DEVICE_ID];
  const wrapped = snapshot.meta[WRAPPED_STORAGE_KEY_META];
  if (
    snapshot.meta[META_USER_ID] !== options.recipientId ||
    !deviceId ||
    !wrapped ||
    !snapshot.account
  ) {
    return null;
  }
  const ciphertext = options.message.ciphertext as { sender_key: string; session_id: string };
  const pickle = await options.backend.readInboundSession(
    inboundSessionKey({
      channel_id: options.message.channel_id,
      sender_key: ciphertext.sender_key,
      session_id: ciphertext.session_id,
    }),
  );
  if (!pickle) return null;
  let storageKey: Uint8Array;
  try {
    storageKey = await openStorageKey({ name: options.name, wrapped, keys: options.keys });
  } catch {
    return null;
  }
  let account: DeviceAccount | null = null;
  const inbound = new InboundSessionStore();
  try {
    account = DeviceAccount.fromPickle(snapshot.account, storageKey, options.recipientId, deviceId);
    inbound.restoreSession(pickle, account, storageKey);
    const decrypted = inbound.decrypt(options.message);
    return notificationBody(decrypted.payload.content.body);
  } catch {
    return null;
  } finally {
    // 秘密鍵を持つ wasm 側のメモリーを、すぐに返す。
    inbound.free();
    account?.free();
    storageKey.fill(0);
  }
}

/** Service Worker の外の世界（テストでは差し替える）。 */
export interface PushDecryptEnvironment<Files = unknown> {
  /** `navigator.locks.request(name, { ifAvailable: true }, callback)`。 */
  withLockIfAvailable<T>(name: string, callback: (held: boolean) => Promise<T>): Promise<T>;
  /**
   * 暗号ストアのファイルを読む（ロックを持ったまま呼ぶ）。なければ `null`。ロックを持つ時間を
   * 短くするため、ここではファイルを読むだけにし、wasm の読み込みや復号はしない。
   */
  readFiles(name: string): Promise<Files | null>;
  /** 読んだファイルを、読むだけの保存先として開く（ロックを返したあとに呼ぶ）。 */
  openSnapshot(files: Files): Promise<SnapshotBackend>;
  /** 暗号ストアを開いているタブに頼む。どのタブも復号できなければ `null`。 */
  askTabs(request: PushDecryptRequest): Promise<string | null>;
  /**
   * このブラウザーで今ログインしているユーザー（タブが置く。`recipient.ts`）。分からなければ
   * `null`。Push の宛先と違えば復号しない。
   */
  currentUser(): Promise<string | null>;
  keys?: WrappingKeyStore;
}

/** 暗号ストアの写しを読む保存先。 */
export interface SnapshotBackend extends Pick<CryptoBackend, "read" | "readInboundSession"> {
  close(): Promise<void>;
}

/** Push の `e2ee` を復号して、通知の本文を返す。復号できなければ `null`。 */
export async function decryptPush<Files>(
  value: unknown,
  env: PushDecryptEnvironment<Files>,
): Promise<string | null> {
  const e2ee = parsePushE2ee(value);
  if (!e2ee) return null;
  // ログアウトせずに別のユーザーでログインし直したあとに、前のユーザー宛ての Push が届いても、
  // 前のユーザーの暗号ストアで本文を出さない。
  if ((await env.currentUser().catch(() => null)) !== e2ee.recipientId) return null;
  for (const namespace of pushNamespaces(e2ee.recipientId)) {
    const name = cryptoDatabaseName(namespace);
    // 暗号ストアを開くタブと同じロック（`cryptoLockName`）。持っているあいだ、タブは開かずに待つ。
    // ロックの中ではファイルを読むだけにする。写しは読んだ時点で閉じているので、復号はロックを
    // 返したあとでよい（wasm の読み込みのあいだ、通知から開いたタブを待たせない）。
    const outcome = await env.withLockIfAvailable(
      `zoo-crypto:${name}`,
      async (held): Promise<{ kind: "busy" } | { kind: "read"; files: Files | null }> =>
        held ? { kind: "read", files: await env.readFiles(name) } : { kind: "busy" },
    );
    if (outcome.kind === "busy") {
      return env.askTabs({ type: PUSH_DECRYPT_REQUEST, namespace, message: e2ee.message });
    }
    if (!outcome.files) continue;
    const backend = await env.openSnapshot(outcome.files);
    try {
      return await decryptFromStore({
        name,
        recipientId: e2ee.recipientId,
        backend,
        message: e2ee.message,
        keys: env.keys,
      });
    } finally {
      await backend.close().catch(() => undefined);
    }
  }
  return null;
}

/**
 * Service Worker の中で、Push の復号を 1 件ずつ行う。
 *
 * 同時に届いた Push を並べて動かすと、1 件目が暗号ストアのロックを持っているあいだ、2 件目は
 * ロックを取れず、開いているタブがあると取り違えてタブに頼む（タブがなければ「新しいメッセージ」の
 * まま出る）。前の処理が終わるのを待ってから、ロックを取り直す。
 *
 * - `timeoutMs` は、前の処理を待つ時間も含めた上限。過ぎたら `null` を返す。
 * - 待つうちに上限を過ぎた処理は、復号を始めない（結果を使わないので）。
 * - 次の処理は、前の処理が上限で諦められても、実際に終わるまで待つ（まだロックを持っていることが
 *   あるため）。ただし止まった処理で後をすべて止めないよう、待つのは前の処理を始めてから
 *   `timeoutMs` まで。
 */
export function serializeDecryption(
  decrypt: (e2ee: unknown) => Promise<string | null>,
  timeoutMs: number,
): (e2ee: unknown) => Promise<string | null> {
  const delay = (ms: number) => new Promise<null>((resolve) => setTimeout(() => resolve(null), ms));
  let previous: Promise<unknown> = Promise.resolve();
  return (e2ee) => {
    const deadline = Date.now() + timeoutMs;
    const run = previous
      .then(() => (Date.now() >= deadline ? null : decrypt(e2ee)))
      .catch(() => null);
    previous = previous.then(() => Promise.race([run, delay(timeoutMs)]));
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<null>((resolve) => {
      timer = setTimeout(() => resolve(null), timeoutMs);
    });
    return Promise.race([run, timeout]).finally(() => clearTimeout(timer));
  };
}
