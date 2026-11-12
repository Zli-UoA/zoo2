import { readFileSync } from "node:fs";

import {
  BackupDecryptionKey,
  BackupEncryptionKey,
  CrossSigningKeys,
  DeviceAccount,
  InboundSessionStore,
  OlmSessions,
  OutboundSession,
  RecoveryKey,
  SsssKey,
  constants,
  initSync,
  newRecoverySlot,
  openKeySlotWithRecoveryKey,
  verifyCrossSigningKey,
  verifyDeviceKeys,
  verifyJsonSignature,
} from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it } from "vite-plus/test";

import { runCryptoOperation, toCryptoFailure } from "./operations";

// Node では fetch で wasm を読めないので、ファイルを直接渡す（ブラウザーは Worker のテスト）。
beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

const ALICE = "00000000-0000-4000-8000-00000000000a";
const BOB = "00000000-0000-4000-8000-00000000000b";
const CHANNEL = "00000000-0000-4000-8000-0000000000c1";

function thrown(fn: () => unknown) {
  try {
    fn();
  } catch (error) {
    return toCryptoFailure(error);
  }
  throw new Error("did not throw");
}

describe("zoo-crypto wasm", () => {
  it("Worker の操作で端末鍵を作り、署名を検証できる", () => {
    const deviceId = crypto.randomUUID();
    const result = runCryptoOperation({ op: "generateDeviceKeys", userId: ALICE, deviceId });
    expect(result.deviceKeys).toMatchObject({ user_id: ALICE, device_id: deviceId });
    expect(result.oneTimeKeyCount).toBe(50);
    const verified = verifyDeviceKeys(result.deviceKeys, ALICE, deviceId);
    expect(verified.curve25519).toBe(result.identityKeys.curve25519);
    expect(verified.ed25519).toBe(result.identityKeys.ed25519);
    // 乱数（getrandom の wasm_js）で、毎回違う鍵になる。
    const again = runCryptoOperation({ op: "generateDeviceKeys", userId: ALICE, deviceId });
    expect(again.identityKeys.ed25519).not.toBe(result.identityKeys.ed25519);
  });

  it("端末アカウントを保存して復元できる", () => {
    const deviceId = crypto.randomUUID();
    const account = new DeviceAccount(ALICE, deviceId);
    const storageKey = crypto.getRandomValues(new Uint8Array(32));
    const pickle = account.pickle(storageKey);
    const restored = DeviceAccount.fromPickle(pickle, storageKey, ALICE, deviceId);
    expect(restored.identityKeys()).toEqual(account.identityKeys());

    const wrongOwner = thrown(() =>
      DeviceAccount.fromPickle(pickle, storageKey, BOB, crypto.randomUUID()),
    );
    expect(wrongOwner).toMatchObject({ name: "ZooCryptoError", code: "PickleOwnerMismatch" });
    const shortKey = thrown(() => account.pickle(new Uint8Array(31)));
    expect(shortKey).toMatchObject({ name: "ZooCryptoError", code: "InvalidArgument" });
  });

  it("Olm で Megolm のセッション鍵を配り、メッセージを復号できる", () => {
    const alice = new DeviceAccount(ALICE, crypto.randomUUID());
    const bob = new DeviceAccount(BOB, crypto.randomUUID());
    bob.generateOneTimeKeys(1);
    const bobKeys = verifyDeviceKeys(bob.deviceKeys(), BOB, bob.deviceId);
    const [oneTimeKey] = Object.values(bob.oneTimeKeys());
    bob.markKeysAsPublished();

    const olm = alice.createOutboundOlmSession(bobKeys, oneTimeKey!);
    const outbound = new OutboundSession(CHANNEL, 3, Date.now());
    const toDevice = alice.encryptRoomKey(olm, bobKeys, outbound.roomKey());

    const decrypted = bob.decryptToDevice(new OlmSessions(), toDevice);
    expect(decrypted.sender).toBe(ALICE);
    const roomKey = decrypted.verifyRoomKey(bob, alice.deviceKeys(), null);
    expect(roomKey.senderVerified).toBe(false);
    const store = new InboundSessionStore();
    expect(store.addRoomKey(roomKey)).toBe("added");

    const messageId = crypto.randomUUID();
    const ciphertext = outbound.encrypt(alice, {
      message_id: messageId,
      revision: 0,
      content: { body: "こんにちは", format: "mfm", kind: "message" },
    });
    const message = store.decrypt({
      channel_id: CHANNEL,
      message_id: messageId,
      user_id: ALICE,
      message_kind: "message",
      epoch: 3,
      revision: 0,
      ciphertext,
    });
    expect(message.payload.content.body).toBe("こんにちは");
    expect(message).toMatchObject({ origin: "olm", sender_verified: false });
    expect(message.revision).toEqual({ status: "latest" });

    // 別の行への移植は破棄する。
    const moved = thrown(() =>
      store.decrypt({
        channel_id: CHANNEL,
        message_id: crypto.randomUUID(),
        user_id: ALICE,
        message_kind: "message",
        epoch: 3,
        revision: 0,
        ciphertext,
      }),
    );
    expect(moved).toMatchObject({ code: "Rejected" });
  });

  it("クロス署名鍵で自分の端末に署名できる", () => {
    const keys = CrossSigningKeys.generate(ALICE);
    const upload = keys.uploadJson() as Record<string, unknown>;
    const master = verifyCrossSigningKey(upload.master_key!, ALICE, "master");
    expect(master).toBe(keys.master.publicKey);

    const device = new DeviceAccount(ALICE, crypto.randomUUID());
    const signature = keys.selfSigning.signDevice(device.deviceKeys());
    const deviceKeys = device.deviceKeys() as { signatures: Record<string, Record<string, string>> };
    deviceKeys.signatures[ALICE]![signature.signer_key_id] = signature.signature;
    verifyJsonSignature(
      deviceKeys,
      ALICE,
      signature.signer_key_id,
      keys.selfSigning.publicKey,
    );
  });

  it("リカバリーキーの鍵スロットで SSSS 鍵を開ける", () => {
    const ssss = SsssKey.generate();
    const recovery = RecoveryKey.generate();
    const slot = newRecoverySlot(ALICE, ssss, recovery);
    expect(slot).toMatchObject({ key_id: ssss.keyId, kind: "recovery" });

    const typed = RecoveryKey.parse(recovery.toDisplay());
    const opened = openKeySlotWithRecoveryKey(slot, ALICE, typed);
    expect(opened.keyId).toBe(ssss.keyId);
    expect(opened.toBytes()).toEqual(ssss.toBytes());

    const wrong = thrown(() => openKeySlotWithRecoveryKey(slot, ALICE, RecoveryKey.generate()));
    expect(wrong).toMatchObject({ name: "ZooCryptoError", code: "KeySlotUnlock" });

    const secret = ssss.encryptSecret("m.megolm_backup.v1", "秘密");
    expect(opened.decryptSecret("m.megolm_backup.v1", secret)).toBe("秘密");
  });

  it("鍵バックアップのセッションを暗号化して復号できる", () => {
    const key = BackupDecryptionKey.generate();
    const data = {
      algorithm: constants().MEGOLM_ALGORITHM,
      channel_id: CHANNEL,
      epoch: 1,
      session_id: "session",
      session_key: "key",
      sender_key: "sender",
      sender_claimed_keys: { ed25519: "ed25519" },
      sender_user_id: ALICE,
      sender_device_id: crypto.randomUUID(),
    };
    const sessionData = new BackupEncryptionKey(key.publicKey).encrypt(data);
    expect(key.decrypt(sessionData)).toEqual(data);
    expect(BackupDecryptionKey.fromBytes(key.toBytes()).publicKey).toBe(key.publicKey);
  });
});
