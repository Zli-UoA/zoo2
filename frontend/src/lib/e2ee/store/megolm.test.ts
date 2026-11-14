import { readFileSync } from "node:fs";

import {
  initSync,
  verifyDeviceKeys,
  type EncryptedMessage,
  type JsonObject,
} from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it } from "vite-plus/test";

import { MemoryCryptoBackend } from "./backend";
import { CryptoStore } from "./cryptoStore";
import type { WrappingKeyStore } from "./storageKey";

beforeAll(() => {
  const wasm = readFileSync(
    new URL("../../../../../crates/zoo-crypto-wasm/pkg/zoo_crypto_wasm_bg.wasm", import.meta.url),
  );
  initSync({ module: wasm });
});

const ALICE = "00000000-0000-4000-8000-00000000000a";
const BOB = "00000000-0000-4000-8000-00000000000b";
const CAROL = "00000000-0000-4000-8000-00000000000c";
const CHANNEL = "00000000-0000-4000-8000-0000000000c1";
const NO_MENTIONS = { user_ids: [], channel: false, here: false };

function memoryKeys(): WrappingKeyStore {
  const keys = new Map<string, CryptoKey>();
  return {
    get: async (name) => keys.get(name),
    put: async (name, key) => {
      keys.set(name, key);
    },
    delete: async (name) => {
      keys.delete(name);
    },
  };
}

async function openStore(userId: string) {
  const backend = new MemoryCryptoBackend();
  const { store } = await CryptoStore.open({
    backend,
    name: `zoo-crypto-${userId}.sqlite`,
    userId,
    keys: memoryKeys(),
    reset: async () => {
      await backend.destroy();
      return backend;
    },
  });
  return { store, backend };
}

/** 受信側の端末の、検証済みの端末鍵と、払い出すワンタイム鍵。 */
function recipient(store: CryptoStore) {
  const request = store.registrationRequest();
  const [keyId, oneTimeKey] = Object.entries(request.one_time_keys)[0]!;
  return {
    device: () => verifyDeviceKeys(request.device_keys, store.userId, store.deviceId),
    claimed: { [store.userId]: { [store.deviceId]: { [keyId]: oneTimeKey } } },
  };
}

/** 送信者が配った to-device を、受信者の暗号ストアで処理する。 */
async function deliver(
  sender: CryptoStore,
  receiver: CryptoStore,
  messages: Record<string, Record<string, JsonObject>>,
  id: string,
) {
  const content = messages[receiver.userId]?.[receiver.deviceId];
  expect(content).toBeDefined();
  return receiver.processToDevice(
    [
      {
        id,
        sender_user_id: sender.userId,
        sender_device_id: sender.deviceId,
        type: "m.room.encrypted",
        content: content!,
      },
    ],
    { [sender.userId]: { [sender.deviceId]: sender.deviceKeys() } },
  );
}

function row(
  sender: CryptoStore,
  messageId: string,
  encrypted: { ciphertext: JsonObject; epoch: number },
  overrides: Partial<EncryptedMessage> = {},
): EncryptedMessage {
  return {
    channel_id: CHANNEL,
    message_id: messageId,
    parent_id: null,
    user_id: sender.userId,
    message_kind: "message",
    epoch: encrypted.epoch,
    revision: 0,
    ciphertext: encrypted.ciphertext,
    ...overrides,
  };
}

function outgoing(messageId: string, body: string) {
  return {
    message_id: messageId,
    parent_id: null,
    revision: 0,
    content: { body, format: "mfm", kind: "message" },
    mentions: NO_MENTIONS,
  };
}

describe("MegolmChannels", () => {
  it("配った端末は復号でき、DB に置く暗号文に平文は入らない", async () => {
    const alice = await openStore(ALICE);
    const bob = await openStore(BOB);
    const bobDevice = recipient(bob.store);

    alice.store.megolm.setDistribution(CHANNEL, 3, [bobDevice.device()]);
    const plan = await alice.store.megolm.start(CHANNEL);
    expect(plan.pending).toBe(1);
    expect(plan.claim).toEqual({ [BOB]: { [bob.store.deviceId]: "signed_curve25519" } });
    const { messages, missing } = await alice.store.megolm.share(CHANNEL, bobDevice.claimed);
    expect(missing).toEqual([]);
    alice.store.megolm.markShared(CHANNEL, [{ userId: BOB, deviceId: bob.store.deviceId }]);
    expect((await deliver(alice.store, bob.store, messages, "1")).roomKeys).toBe(1);

    const encrypted = alice.store.megolm.encrypt(CHANNEL, outgoing("m1", "secret hello"));
    expect(encrypted.epoch).toBe(3);
    expect(JSON.stringify(encrypted.ciphertext)).not.toContain("secret hello");

    const [received] = await bob.store.megolm.decrypt([row(alice.store, "m1", encrypted)]);
    // クロス署名をたどれない送信端末は「未検証の送信者」として表示する。
    expect(received).toMatchObject({
      status: "decrypted",
      body: "secret hello",
      sender: "unverified",
    });
    // 送信者自身も、自分のメッセージを読める。
    const [own] = await alice.store.megolm.decrypt([row(alice.store, "m1", encrypted)]);
    expect(own).toMatchObject({ status: "decrypted", body: "secret hello" });

    // 配った端末にはもう配らない。
    expect((await alice.store.megolm.start(CHANNEL)).pending).toBe(0);
    await alice.store.close();
    await bob.store.close();
  });

  it("行と照合できない暗号文は破棄する（種別の入れ替え、別の行への移植）", async () => {
    const alice = await openStore(ALICE);
    const bob = await openStore(BOB);
    const bobDevice = recipient(bob.store);
    alice.store.megolm.setDistribution(CHANNEL, 1, [bobDevice.device()]);
    await alice.store.megolm.start(CHANNEL);
    const { messages } = await alice.store.megolm.share(CHANNEL, bobDevice.claimed);
    await deliver(alice.store, bob.store, messages, "1");
    const encrypted = alice.store.megolm.encrypt(CHANNEL, outgoing("m1", "hello"));

    const results = await bob.store.megolm.decrypt([
      row(alice.store, "m1", encrypted, { message_kind: "emote" }),
      row(alice.store, "m2", encrypted),
      row(alice.store, "m1", encrypted, { user_id: BOB }),
      row(alice.store, "m1", encrypted, { epoch: 2 }),
    ]);
    for (const result of results)
      expect(result).toEqual({ status: "undecryptable", reason: "invalid" });
    await alice.store.close();
    await bob.store.close();
  });

  it("退出したメンバーは、epoch が変わったあとのメッセージを復号できない", async () => {
    const alice = await openStore(ALICE);
    const bob = await openStore(BOB);
    const carol = await openStore(CAROL);
    const bobDevice = recipient(bob.store);
    const carolDevice = recipient(carol.store);

    alice.store.megolm.setDistribution(CHANNEL, 1, [bobDevice.device(), carolDevice.device()]);
    await alice.store.megolm.start(CHANNEL);
    const claimed = { ...bobDevice.claimed, ...carolDevice.claimed };
    const first = await alice.store.megolm.share(CHANNEL, claimed);
    await deliver(alice.store, bob.store, first.messages, "1");
    await deliver(alice.store, carol.store, first.messages, "1");

    // Carol が退出して epoch が 2 になった。Shape の値と違うので、送信用セッションを作り直す。
    expect(alice.store.megolm.needsSnapshot(CHANNEL, 2)).toBe(true);
    alice.store.megolm.setDistribution(CHANNEL, 2, [bobDevice.device()]);
    const plan = await alice.store.megolm.start(CHANNEL);
    // Bob とは送信用の Olm セッションがあるので、ワンタイム鍵を払い出さずに使い回す。
    expect(plan).toEqual({ claim: {}, pending: 1 });
    const second = await alice.store.megolm.share(CHANNEL, {});
    expect(second.messages[CAROL]).toBeUndefined();
    await deliver(alice.store, bob.store, second.messages, "2");

    const encrypted = alice.store.megolm.encrypt(CHANNEL, outgoing("m2", "after leave"));
    expect(encrypted.epoch).toBe(2);
    const message = row(alice.store, "m2", encrypted);
    expect((await bob.store.megolm.decrypt([message]))[0]).toMatchObject({
      status: "decrypted",
      body: "after leave",
    });
    expect((await carol.store.megolm.decrypt([message]))[0]).toEqual({
      status: "undecryptable",
      reason: "keyMissing",
    });
    await alice.store.close();
    await bob.store.close();
    await carol.store.close();
  });

  it("払い出された鍵がない端末は、配れなかった端末として返す", async () => {
    const alice = await openStore(ALICE);
    const bob = await openStore(BOB);
    alice.store.megolm.setDistribution(CHANNEL, 1, [recipient(bob.store).device()]);
    await alice.store.megolm.start(CHANNEL);
    const { messages, missing } = await alice.store.megolm.share(CHANNEL, {});
    expect(messages).toEqual({});
    expect(missing).toEqual([{ userId: BOB, deviceId: bob.store.deviceId }]);
    await alice.store.close();
    await bob.store.close();
  });

  it("開き直しても、自分が送ったメッセージを読める（送信用セッションは作り直す）", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const options = {
      backend,
      name: "zoo-crypto-own.sqlite",
      userId: ALICE,
      keys,
      reset: async () => backend,
    };
    const bob = await openStore(BOB);
    const first = await CryptoStore.open(options);
    first.store.megolm.setDistribution(CHANNEL, 1, [recipient(bob.store).device()]);
    await first.store.megolm.start(CHANNEL);
    const encrypted = first.store.megolm.encrypt(CHANNEL, outgoing("m1", "before reload"));
    const message = row(first.store, "m1", encrypted);
    await first.store.close();

    const second = await CryptoStore.open(options);
    // 自分の送信用セッションの受信用の写しは暗号ストアにある（鍵バックアップの対象にも入る）。
    expect(await second.store.pendingBackupCount()).toBe(1);
    expect((await second.store.megolm.decrypt([message]))[0]).toMatchObject({
      status: "decrypted",
      body: "before reload",
    });
    // 送信用セッションはメモリーにしかないので、次の送信の前に作り直す。
    expect(second.store.megolm.needsSnapshot(CHANNEL, 1)).toBe(true);
    await second.store.close();
    await bob.store.close();
  });

  it("一度暗号化と見たチャンネルを、開き直しても覚えている", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const options = {
      backend,
      name: "zoo-crypto-remember.sqlite",
      userId: ALICE,
      keys,
      reset: async () => backend,
    };
    const first = await CryptoStore.open(options);
    expect(await first.store.megolm.isKnownEncrypted(CHANNEL)).toBe(false);
    await first.store.megolm.rememberEncrypted(CHANNEL);
    await first.store.close();
    const second = await CryptoStore.open(options);
    expect(await second.store.megolm.isKnownEncrypted(CHANNEL)).toBe(true);
    expect(await second.store.megolm.isKnownEncrypted("other")).toBe(false);
    await second.store.close();
  });

  it("前回の配布先から増えたメンバーを、開き直しても返す（初めてのチャンネルは空）", async () => {
    const backend = new MemoryCryptoBackend();
    const keys = memoryKeys();
    const options = {
      backend,
      name: "zoo-crypto-recipients.sqlite",
      userId: ALICE,
      keys,
      reset: async () => backend,
    };
    const first = await CryptoStore.open(options);
    expect(await first.store.megolm.noteRecipients(CHANNEL, [BOB, ALICE])).toEqual([]);
    expect(await first.store.megolm.noteRecipients(CHANNEL, [ALICE, BOB])).toEqual([]);
    await first.store.close();
    const second = await CryptoStore.open(options);
    expect(await second.store.megolm.noteRecipients(CHANNEL, [ALICE, BOB, CAROL])).toEqual([CAROL]);
    expect(await second.store.megolm.noteRecipients("other", [ALICE, CAROL])).toEqual([]);
    await second.store.close();
  });

  it("解錠していない端末は、配布先を判定せず NeedsUnlock で失敗する", async () => {
    const alice = await openStore(ALICE);
    await expect(
      alice.store.megolm.evaluate(CHANNEL, [
        { channel_id: CHANNEL, membership_epoch: 1, user_ids: [ALICE], device_keys: {} },
      ]),
    ).rejects.toMatchObject({ code: "NeedsUnlock" });
    await alice.store.close();
  });
});
