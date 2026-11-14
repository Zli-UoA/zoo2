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

export const ENCRYPTED_CHANNELS_STORAGE_KEY = "zoo.e2ee.encryptedChannels";

function storedEncryptedChannels(): string[] {
  try {
    const value: unknown = JSON.parse(readStorageValue(ENCRYPTED_CHANNELS_STORAGE_KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((id): id is string => typeof id === "string") : [];
  } catch {
    return [];
  }
}

/** `encrypted = true` を見た。描画の中で呼んでよい（同じ値を足すだけ）。 */
export function noteEncryptedChannel(channelId: string): void {
  if (seenEncrypted.has(channelId)) return;
  seenEncrypted.add(channelId);
  const stored = storedEncryptedChannels();
  if (!stored.includes(channelId)) {
    writeStorageValue(ENCRYPTED_CHANNELS_STORAGE_KEY, JSON.stringify([...stored, channelId]));
  }
}

function seenAsEncrypted(channelId: string): boolean {
  if (seenEncrypted.has(channelId)) return true;
  // ほかのタブや、開き直す前に覚えたもの。
  if (!storedEncryptedChannels().includes(channelId)) return false;
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
