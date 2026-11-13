/**
 * 暗号化の格下げを防ぐための判定（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
 *
 * 一度 `encrypted = true` を見たチャンネルは、このページを開いているあいだメモリーに覚える
 * （暗号ストアの記録を Worker に問い合わせる前でも、同期的に格下げとして扱うため）。
 */
const seenEncrypted = new Set<string>();

/** `encrypted = true` を見た。描画の中で呼んでよい（同じ値を足すだけ）。 */
export function noteEncryptedChannel(channelId: string): void {
  seenEncrypted.add(channelId);
}

/** テスト用：覚えたチャンネルを忘れる。 */
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
    seenEncrypted.has(input.channelId) ||
    input.known === true ||
    input.channelName?.startsWith("edm:") === true ||
    input.hasCiphertext;
  return {
    downgraded,
    checking: !downgraded && (input.known === "pending" || input.known === "failed"),
  };
}
