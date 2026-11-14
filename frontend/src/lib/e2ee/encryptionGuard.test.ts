import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import {
  ENCRYPTED_CHANNEL_STORAGE_PREFIX,
  encryptionSendState,
  noteEncryptedChannel,
  resetSeenEncryptedChannels,
  subscribeEncryptedChannels,
} from "./encryptionGuard";

const base = { channelId: "c1", hasCiphertext: false } as const;

beforeEach(() => resetSeenEncryptedChannels());
afterEach(() => vi.unstubAllGlobals());

/** `window.localStorage` を Map で置き換える。 */
function stubLocalStorage() {
  const values = new Map<string, string>();
  vi.stubGlobal("window", {
    localStorage: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    },
  });
  return values;
}

describe("encryptionSendState", () => {
  it("暗号化チャンネルは格下げでも確認中でもない", () => {
    expect(encryptionSendState({ ...base, encrypted: true, known: "pending" })).toEqual({
      downgraded: false,
      checking: false,
    });
  });

  it("このページで暗号化と見たチャンネルは、記録の問い合わせを待たずに格下げとして扱う", () => {
    noteEncryptedChannel("c1");
    for (const known of ["pending", "failed", "unavailable", false] as const) {
      expect(encryptionSendState({ ...base, encrypted: false, known })).toEqual({
        downgraded: true,
        checking: false,
      });
    }
  });

  it("暗号化と見たチャンネルは、開き直したあと暗号ストアのないタブでも格下げとして扱う", () => {
    const values = stubLocalStorage();
    noteEncryptedChannel("c1");
    expect(values.get(`${ENCRYPTED_CHANNEL_STORAGE_PREFIX}c1`)).toBe("1");
    // 開き直した（メモリーの記録はない）。暗号ストアの記録も問い合わせられない。
    resetSeenEncryptedChannels();
    expect(encryptionSendState({ ...base, encrypted: false, known: "unavailable" })).toEqual({
      downgraded: true,
      checking: false,
    });
    expect(
      encryptionSendState({ ...base, channelId: "c2", encrypted: false, known: "unavailable" })
        .downgraded,
    ).toBe(false);
  });

  it("記録、edm: の名前、暗号文のある行のどれかで格下げとして扱う", () => {
    expect(
      encryptionSendState({ ...base, encrypted: false, known: false, channelName: "edm:a:b" })
        .downgraded,
    ).toBe(true);
    expect(
      encryptionSendState({ ...base, encrypted: false, known: false, hasCiphertext: true })
        .downgraded,
    ).toBe(true);
    // 記録で分かったチャンネルは覚えるので、最後に確かめる。
    expect(encryptionSendState({ ...base, encrypted: false, known: true }).downgraded).toBe(true);
  });

  it("暗号ストアの記録で暗号化と分かったチャンネルは、暗号ストアのないタブにも覚えさせる", () => {
    const values = stubLocalStorage();
    expect(encryptionSendState({ ...base, encrypted: false, known: true }).downgraded).toBe(true);
    expect(values.get(`${ENCRYPTED_CHANNEL_STORAGE_PREFIX}c1`)).toBe("1");
    // 開き直した、暗号ストアを開いていないタブ。
    resetSeenEncryptedChannels();
    expect(encryptionSendState({ ...base, encrypted: false, known: "unavailable" })).toEqual({
      downgraded: true,
      checking: false,
    });
    // 記録がないと分かっただけなら、覚えない。
    encryptionSendState({ ...base, channelId: "c2", encrypted: false, known: false });
    expect(values.has(`${ENCRYPTED_CHANNEL_STORAGE_PREFIX}c2`)).toBe(false);
  });

  it("記録の問い合わせの応答待ちと失敗のあいだは、平文でも送らない", () => {
    expect(encryptionSendState({ ...base, encrypted: false, known: "pending" })).toEqual({
      downgraded: false,
      checking: true,
    });
    expect(encryptionSendState({ ...base, encrypted: false, known: "failed" }).checking).toBe(true);
    // 暗号ストアがないタブは問い合わせられないので、ほかの手がかり（localStorage の記録を含む）だけで判定する。
    expect(encryptionSendState({ ...base, encrypted: false, known: "unavailable" })).toEqual({
      downgraded: false,
      checking: false,
    });
    expect(encryptionSendState({ ...base, encrypted: false, known: false })).toEqual({
      downgraded: false,
      checking: false,
    });
  });
});

describe("subscribeEncryptedChannels", () => {
  it("新しく覚えたときだけ、描画のあとに知らせる", async () => {
    stubLocalStorage();
    const listener = vi.fn();
    const unsubscribe = subscribeEncryptedChannels(listener);
    noteEncryptedChannel("c-new");
    // 呼んだその場では知らせない（描画の中で呼ばれても、ほかの部品を更新しない）。
    expect(listener).not.toHaveBeenCalled();
    await Promise.resolve();
    expect(listener).toHaveBeenCalledTimes(1);
    noteEncryptedChannel("c-new");
    await Promise.resolve();
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
  });
});
