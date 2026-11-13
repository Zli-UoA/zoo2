import { beforeEach, describe, expect, it } from "vite-plus/test";

import {
  encryptionSendState,
  noteEncryptedChannel,
  resetSeenEncryptedChannels,
} from "./encryptionGuard";

const base = { channelId: "c1", hasCiphertext: false } as const;

beforeEach(() => resetSeenEncryptedChannels());

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

  it("記録、edm: の名前、暗号文のある行のどれかで格下げとして扱う", () => {
    expect(encryptionSendState({ ...base, encrypted: false, known: true }).downgraded).toBe(true);
    expect(
      encryptionSendState({ ...base, encrypted: false, known: false, channelName: "edm:a:b" })
        .downgraded,
    ).toBe(true);
    expect(
      encryptionSendState({ ...base, encrypted: false, known: false, hasCiphertext: true })
        .downgraded,
    ).toBe(true);
  });

  it("記録の問い合わせの応答待ちと失敗のあいだは、平文でも送らない", () => {
    expect(encryptionSendState({ ...base, encrypted: false, known: "pending" })).toEqual({
      downgraded: false,
      checking: true,
    });
    expect(encryptionSendState({ ...base, encrypted: false, known: "failed" }).checking).toBe(true);
    // 暗号ストアがないタブは問い合わせられないので、ほかの手がかりだけで判定する。
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
