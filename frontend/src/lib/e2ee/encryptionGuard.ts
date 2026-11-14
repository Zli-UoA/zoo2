import { readStorageValue, writeStorageValue } from "../storage";

/**
 * 暗号化の格下げを防ぐための判定（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
 *
 * 一度 `encrypted = true` を見たチャンネルは、メモリーと `localStorage` に覚える。暗号ストアの記録を
 * Worker に問い合わせる前でも同期的に格下げとして扱うためと、暗号ストアを開いていないタブ（記録を
 * 問い合わせられない）でも、開き直したあとまで格下げとして扱うため。覚えるのはチャンネルの ID だけで、
 * サーバーも知っている値なので、暗号化しない。
 */
const seenEncrypted = new Set<string>();

/**
 * チャンネルごとのキーの接頭辞。チャンネルごとに別のキーに置くのは、2 つのタブが別のチャンネルを
 * 同時に覚えたときに、一覧を読んで書き戻すと片方の記録が消えるため（`localStorage` には読み書きの
 * 排他がない）。値は消さないので、どの順で書いても記録は残る。
 */
export const ENCRYPTED_CHANNEL_STORAGE_PREFIX = "zoo.e2ee.encryptedChannel:";

function storageKey(channelId: string): string {
  return `${ENCRYPTED_CHANNEL_STORAGE_PREFIX}${channelId}`;
}

/** `encrypted = true` を見た。描画の中で呼んでよい（同じ値を足すだけ）。 */
export function noteEncryptedChannel(channelId: string): void {
  if (seenEncrypted.has(channelId)) return;
  seenEncrypted.add(channelId);
  if (readStorageValue(storageKey(channelId)) !== "1")
    writeStorageValue(storageKey(channelId), "1");
}

function seenAsEncrypted(channelId: string): boolean {
  if (seenEncrypted.has(channelId)) return true;
  // ほかのタブや、開き直す前に覚えたもの。
  if (readStorageValue(storageKey(channelId)) !== "1") return false;
  seenEncrypted.add(channelId);
  return true;
}

/** テスト用：メモリーに覚えたチャンネルを忘れる（開き直しに当たる。`localStorage` は消さない）。 */
export function resetSeenEncryptedChannels(): void {
  seenEncrypted.clear();
}

/**
 * 暗号ストアの記録の問い合わせの状態。`true`/`false` は結果、`pending` は応答待ち、`failed` は失敗、
 * `unavailable` はこのタブに暗号ストアがない（問い合わせられない）。
 */
export type KnownEncryptedState = boolean | "pending" | "failed" | "unavailable";

export interface EncryptionSendState {
  /** 暗号化チャンネルだったのに `encrypted = false` が届いた。平文で送らず、警告する。 */
  downgraded: boolean;
  /** 暗号ストアの記録を確かめている（または確かめられなかった）。確かめ終わるまで送らない。 */
  checking: boolean;
}

export function encryptionSendState(input: {
  channelId: string;
  encrypted: boolean;
  known: KnownEncryptedState;
  channelName?: string;
  hasCiphertext: boolean;
}): EncryptionSendState {
  if (input.encrypted) return { downgraded: false, checking: false };
  // 暗号ストアの記録で暗号化と分かったら、`localStorage` にも覚える（暗号ストアを開いていないタブでも、
  // 開き直したあとまで格下げとして扱えるように）。
  if (input.known === true) noteEncryptedChannel(input.channelId);
  const downgraded =
    seenAsEncrypted(input.channelId) ||
    input.known === true ||
    input.channelName?.startsWith("edm:") === true ||
    input.hasCiphertext;
  return {
    downgraded,
    checking: !downgraded && (input.known === "pending" || input.known === "failed"),
  };
}
