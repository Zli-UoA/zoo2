import type { EncryptedMessage, JsonObject } from "@zoo/crypto-wasm";

/**
 * 暗号化チャンネルの Web Push を Service Worker で復号する（docs/e2ee.md の「Web Push の復号」、#365）。
 *
 * サーバーは本文を「新しいメッセージ」にし、メッセージの行（Megolm の暗号文と、受信時の検証に使う列）
 * を `e2ee` に付けて送る（`backend/src/push.rs`）。Service Worker は次のどちらかで復号する。
 *
 * - 暗号ストアを開いているタブがない（Web Locks を取れた）：ロックを持ったまま、OPFS の暗号ストアの
 *   ファイルの写しを読み、受信用セッションで復号する。写しは書き戻さない。
 * - 開いているタブがある：そのタブに頼む（{@link PUSH_DECRYPT_REQUEST}）。タブの Worker が、画面の
 *   表示と同じ `megolmDecrypt` で復号する。
 *
 * どちらでも復号できなければ、サーバーの本文（「新しいメッセージ」）のまま出す。
 */

/** Service Worker からタブへの、復号の依頼の `type`。 */
export const PUSH_DECRYPT_REQUEST = "zoo-e2ee-push-decrypt";

/** 通知に出す本文の長さ（平文チャンネルの Push と同じ、先頭 240 文字）。 */
export const PUSH_BODY_MAX_CHARS = 240;

/** タブの応答を待つ時間。過ぎたら「新しいメッセージ」のまま出す。 */
export const PUSH_TAB_TIMEOUT_MS = 3_000;

/** Push の `e2ee`。 */
export interface PushE2ee {
  /** 通知の宛先のユーザー。この人の暗号ストアで復号する。 */
  recipientId: string;
  /** メッセージの行（zoo-crypto の `EncryptedMessage` と同じ形）。 */
  message: EncryptedMessage;
}

/** タブへの依頼。`namespace` の暗号ストアを開いているタブだけが復号して返す。 */
export interface PushDecryptRequest {
  type: typeof PUSH_DECRYPT_REQUEST;
  namespace: string;
  message: EncryptedMessage;
}

/** タブの応答。復号できなければ `body` は `null`。 */
export interface PushDecryptReply {
  body: string | null;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

/**
 * メッセージの行を確かめる。形が違えば `null`。中身の照合（チャンネル、行、送信者、epoch など）は
 * zoo-crypto の受信時の検証が行う。
 */
export function parseEncryptedMessage(value: unknown): EncryptedMessage | null {
  if (!isObject(value)) return null;
  const { channel_id, message_id, parent_id, user_id, message_kind, epoch, revision, ciphertext } =
    value;
  if (
    typeof channel_id !== "string" ||
    typeof message_id !== "string" ||
    (parent_id != null && typeof parent_id !== "string") ||
    typeof user_id !== "string" ||
    typeof message_kind !== "string" ||
    !isCount(epoch) ||
    !isCount(revision) ||
    !isObject(ciphertext) ||
    typeof ciphertext.sender_key !== "string" ||
    typeof ciphertext.session_id !== "string"
  ) {
    return null;
  }
  return {
    channel_id,
    message_id,
    parent_id: parent_id ?? null,
    user_id,
    message_kind,
    epoch,
    revision,
    ciphertext: ciphertext as JsonObject,
  };
}

/** Push の `e2ee` を確かめる。なければ、または形が違えば `null`（復号せずに出す）。 */
export function parsePushE2ee(value: unknown): PushE2ee | null {
  if (!isObject(value)) return null;
  const recipientId = value.recipient_id;
  const message = parseEncryptedMessage(value.message);
  if (typeof recipientId !== "string" || !UUID.test(recipientId) || !message) return null;
  return { recipientId, message };
}

/**
 * 宛先のユーザーの暗号ストアの名前空間の候補（`resolveOfflineNamespace` と同じ形）。OIDC は
 * `user-`、開発用のログインは `dev-` なので、両方を順に試す。
 */
export function pushNamespaces(recipientId: string): string[] {
  return [`user-${recipientId}`, `dev-${recipientId}`];
}

/** 通知に出す本文。サーバーの `chars().take(240)` と同じく、コードポイントで数える。 */
export function notificationBody(body: string): string {
  return Array.from(body).slice(0, PUSH_BODY_MAX_CHARS).join("");
}

/** タブの Worker（`CryptoWorker` の `megolmDecrypt` だけを使う）。 */
export interface PushDecryptWorker {
  request(operation: {
    op: "megolmDecrypt";
    rows: EncryptedMessage[];
  }): Promise<readonly ({ status: "decrypted"; body: string } | { status: "undecryptable" })[]>;
}

/**
 * タブで、Service Worker の依頼に答える。`worker` は、このタブが開いている暗号ストアの Worker
 * （開いていなければ `null`）。依頼でなければ `false` を返す。
 */
export function answerPushDecryptRequest(
  data: unknown,
  port: MessagePort | undefined,
  current: { worker: PushDecryptWorker | null; namespace: string },
): boolean {
  if (!isObject(data) || data.type !== PUSH_DECRYPT_REQUEST || !port) return false;
  const reply = (body: string | null) => {
    try {
      port.postMessage({ body } satisfies PushDecryptReply);
    } catch {
      // Service Worker が止まった。
    }
  };
  const message = parseEncryptedMessage(data.message);
  if (!current.worker || data.namespace !== current.namespace || !message) {
    reply(null);
    return true;
  }
  void current.worker
    .request({ op: "megolmDecrypt", rows: [message] })
    .then(([outcome]) =>
      reply(outcome?.status === "decrypted" ? notificationBody(outcome.body) : null),
    )
    .catch(() => reply(null));
  return true;
}
