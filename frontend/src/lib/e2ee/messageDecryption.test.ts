import { describe, expect, it } from "vite-plus/test";

import type { MegolmCiphertext } from "../electric/types";
import { isEncryptedMessage, resolveMessageDecryption } from "./messageDecryption";

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
});
