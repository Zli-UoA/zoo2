import { describe, expect, it } from "vite-plus/test";

import type { MegolmCiphertext } from "../electric/types";
import {
  isEncryptedMessage,
  resolveMessageDecryption,
  applyRevision,
  type LatestRevisions,
  type MessageDecryption,
} from "./messageDecryption";

const ciphertext: MegolmCiphertext = {
  algorithm: "m.megolm.v1.aes-sha2",
  sender_key: "sender",
  device_id: "device",
  session_id: "session",
  ciphertext: "AA",
};

describe("resolveMessageDecryption", () => {
  it("平文のメッセージは null を返す", () => {
    expect(isEncryptedMessage({ ciphertext: null })).toBe(false);
    expect(resolveMessageDecryption({ ciphertext: null }, undefined)).toBeNull();
    expect(resolveMessageDecryption({}, undefined)).toBeNull();
  });

  it("復号の結果がまだない暗号文は locked にする", () => {
    expect(resolveMessageDecryption({ ciphertext }, undefined)).toEqual({
      status: "undecryptable",
      reason: "locked",
    });
  });

  it("渡された復号の結果をそのまま使う", () => {
    const decrypted = {
      status: "decrypted",
      body: "hi",
      kind: "message",
      sender: "unverified",
    } as const;
    expect(resolveMessageDecryption({ ciphertext }, decrypted)).toBe(decrypted);
  });

  it("暗号化チャンネルの暗号文のない行は unencrypted のまま出す（本文を出さない）", () => {
    const unencrypted: MessageDecryption = { status: "undecryptable", reason: "unencrypted" };
    expect(resolveMessageDecryption({ ciphertext: null }, unencrypted)).toBe(unencrypted);
    // 暗号文のない行に、ほかの復号の結果は当てない。
    expect(
      resolveMessageDecryption(
        { ciphertext: null },
        { status: "decrypted", body: "x", kind: "message", sender: "verified" },
      ),
    ).toBeNull();
  });
});

describe("applyRevision", () => {
  const v0: MessageDecryption = {
    status: "decrypted",
    body: "before",
    kind: "message",
    sender: "verified",
  };
  const v1: MessageDecryption = { ...v0, body: "after" };

  it("新しい版を見たあとに古い版の行が届いたら、最新の版の本文のまま警告する", () => {
    const latest: LatestRevisions = new Map();
    expect(applyRevision(latest, "m1", 0, v0, "latest")).toBe(v0);
    expect(applyRevision(latest, "m1", 1, v1, "latest")).toBe(v1);
    // 前に復号した版 0 の結果を使い回した（Worker を通さない）ときも、照合を飛ばさない。
    expect(applyRevision(latest, "m1", 0, v0, "latest")).toEqual({
      ...v1,
      revisionWarning: "stale",
    });
  });

  it("Worker が stale と返したら、知らない最新の版の代わりにこの本文で警告する", () => {
    expect(applyRevision(new Map(), "m1", 0, v0, "stale")).toEqual({
      ...v0,
      revisionWarning: "stale",
    });
  });

  it("同じ版で中身が違えば、先に見た方の本文のまま警告する", () => {
    const latest: LatestRevisions = new Map();
    applyRevision(latest, "m1", 1, v1, "latest");
    expect(applyRevision(latest, "m1", 1, { ...v1, body: "forged" }, "conflict")).toEqual({
      ...v1,
      revisionWarning: "conflict",
    });
    // 最新の版の記録は変えない。
    expect(latest.get("m1")?.decryption).toBe(v1);
  });

  it("復号できない行はそのまま返す", () => {
    const failed: MessageDecryption = { status: "undecryptable", reason: "invalid" };
    expect(applyRevision(new Map(), "m1", 0, failed, "latest")).toBe(failed);
  });
});
