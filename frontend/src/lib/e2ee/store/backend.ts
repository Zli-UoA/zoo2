/**
 * 暗号ストアの保存先（docs/e2ee.md の「暗号ストア」）。
 *
 * 値はすべて zoo-crypto が暗号化した文字列（端末アカウントは `DeviceAccount.pickle`、ほかは
 * `zoo-crypto` の保存用の封筒）で、保存先は中身を読まない。例外は `meta` の値で、秘密を入れない
 * （包んだ保存鍵、`device_id`、登録済みかどうか）。
 *
 * ブラウザーでは OPFS の SQLite（`sqliteBackend.ts`）、Node のテストではメモリー
 * （{@link MemoryCryptoBackend}）を使う。
 */

export interface StoredOlmSession {
  sessionId: string;
  theirIdentityKey: string;
  pickle: string;
}

/** 受信用セッションのキー（`SessionRef`）を 1 つの文字列にしたもの。 */
export type InboundSessionKey = string;

export function inboundSessionKey(ref: {
  channel_id: string;
  sender_key: string;
  session_id: string;
}): InboundSessionKey {
  return JSON.stringify([ref.channel_id, ref.sender_key, ref.session_id]);
}

/** 1 回で書く変更。{@link CryptoBackend.write} はこれを 1 つのトランザクションで書く。 */
export interface CryptoStoreWrite {
  /** `null` の値は消す。 */
  meta?: Record<string, string | null>;
  account?: string;
  olmSessions?: StoredOlmSession[];
  /** 受信用セッション。新しく足したものや置き換えたものは、鍵バックアップの送信待ちに入れ直す。 */
  inboundSessions?: { key: InboundSessionKey; pickle: string }[];
  trustStore?: string;
  /** 処理を終えて、まだ ack を送れていない to-device メッセージの ID。 */
  addProcessedToDevice?: string[];
  /** ack を送れた ID。 */
  removeProcessedToDevice?: string[];
}

export interface CryptoStoreSnapshot {
  meta: Record<string, string>;
  account: string | null;
  olmSessions: StoredOlmSession[];
  trustStore: string | null;
  processedToDevice: string[];
}

export interface CryptoBackend {
  /** 起動時に読むもの（受信用セッションは数が多いので、使うときに {@link readInboundSession} で読む）。 */
  read(): Promise<CryptoStoreSnapshot>;
  readInboundSession(key: InboundSessionKey): Promise<string | null>;
  write(changes: CryptoStoreWrite): Promise<void>;
  /** 鍵バックアップにまだ送っていない受信用セッションの数。 */
  pendingBackupCount(): Promise<number>;
  close(): Promise<void>;
  /** 閉じて、保存先を消す。 */
  destroy(): Promise<void>;
}

/** テスト用のメモリーの保存先。 */
export class MemoryCryptoBackend implements CryptoBackend {
  meta = new Map<string, string>();
  account: string | null = null;
  olmSessions = new Map<string, StoredOlmSession>();
  inboundSessions = new Map<InboundSessionKey, { pickle: string; backedUp: boolean }>();
  trustStore: string | null = null;
  processedToDevice = new Set<string>();
  destroyed = false;

  async read(): Promise<CryptoStoreSnapshot> {
    return {
      meta: Object.fromEntries(this.meta),
      account: this.account,
      olmSessions: [...this.olmSessions.values()],
      trustStore: this.trustStore,
      processedToDevice: [...this.processedToDevice],
    };
  }

  async readInboundSession(key: InboundSessionKey): Promise<string | null> {
    return this.inboundSessions.get(key)?.pickle ?? null;
  }

  async write(changes: CryptoStoreWrite): Promise<void> {
    for (const [key, value] of Object.entries(changes.meta ?? {})) {
      if (value === null) this.meta.delete(key);
      else this.meta.set(key, value);
    }
    if (changes.account !== undefined) this.account = changes.account;
    for (const session of changes.olmSessions ?? []) {
      this.olmSessions.set(session.sessionId, session);
    }
    for (const { key, pickle } of changes.inboundSessions ?? []) {
      this.inboundSessions.set(key, { pickle, backedUp: false });
    }
    if (changes.trustStore !== undefined) this.trustStore = changes.trustStore;
    for (const id of changes.addProcessedToDevice ?? []) this.processedToDevice.add(id);
    for (const id of changes.removeProcessedToDevice ?? []) this.processedToDevice.delete(id);
  }

  async pendingBackupCount(): Promise<number> {
    return [...this.inboundSessions.values()].filter((session) => !session.backedUp).length;
  }

  async close(): Promise<void> {}

  async destroy(): Promise<void> {
    this.destroyed = true;
    this.meta.clear();
    this.account = null;
    this.olmSessions.clear();
    this.inboundSessions.clear();
    this.trustStore = null;
    this.processedToDevice.clear();
  }
}
