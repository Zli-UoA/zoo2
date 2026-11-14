import { readFileSync } from "node:fs";

import {
  OwnSecrets,
  initSync,
  verifyDeviceKeys,
  type EncryptedMessage,
  type JsonObject,
} from "@zoo/crypto-wasm";
import { beforeAll, describe, expect, it } from "vite-plus/test";

import { MemoryCryptoBackend } from "./backend";
import { CryptoStore, type KeysQueryResponse } from "./cryptoStore";
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
  keysQuery: KeysQueryResponse = keysQueryOf({
    [sender.userId]: { [sender.deviceId]: sender.deviceKeys() },
  }),
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
    keysQuery,
  );
}

/** 端末鍵だけの `keys/query` の応答（クロス署名鍵がないので、送信端末は検証済みにならない）。 */
function keysQueryOf(deviceKeys: Record<string, Record<string, JsonObject>>): KeysQueryResponse {
  return {
    device_keys: deviceKeys,
    master_keys: {},
    self_signing_keys: {},
    user_signing_keys: {},
    tofu_pins: {},
    signatures: [],
  };
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
    // 復号した平文の mentions も返す（受信側の表示とハイライトはこれだけを使う）。
    expect(received).toMatchObject({ mentions: NO_MENTIONS });
    // 送信者自身も、自分のメッセージを読める。
    const [own] = await alice.store.megolm.decrypt([row(alice.store, "m1", encrypted)]);
    expect(own).toMatchObject({ status: "decrypted", body: "secret hello" });

    // 配った端末にはもう配らない。
    expect((await alice.store.megolm.start(CHANNEL)).pending).toBe(0);
    await alice.store.close();
    await bob.store.close();
  });

  it("復号の結果に、暗号化した平文の mentions をそのまま返す", async () => {
    const alice = await openStore(ALICE);
    const bob = await openStore(BOB);
    const bobDevice = recipient(bob.store);
    alice.store.megolm.setDistribution(CHANNEL, 1, [bobDevice.device()]);
    await alice.store.megolm.start(CHANNEL);
    const { messages } = await alice.store.megolm.share(CHANNEL, bobDevice.claimed);
    await deliver(alice.store, bob.store, messages, "1");
    const mentions = { user_ids: [BOB, CAROL], channel: true, here: false };
    const encrypted = alice.store.megolm.encrypt(CHANNEL, {
      ...outgoing("m1", "@bob @carol @channel"),
      mentions,
    });

    const [received] = await bob.store.megolm.decrypt([row(alice.store, "m1", encrypted)]);
    expect(received).toMatchObject({ status: "decrypted", mentions });
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

/** ユーザーのクロス署名鍵（`OwnSecrets`）と、`keys/query` に載せる公開鍵。 */
function identity(userId: string) {
  const secrets = OwnSecrets.generate(userId);
  const cs = secrets.crossSigning;
  const upload = cs.uploadJson() as Record<string, JsonObject>;
  return {
    secrets,
    master: upload.master_key!,
    selfSigning: upload.self_signing_key!,
    userSigning: upload.user_signing_key!,
    /**
     * 自己署名鍵で端末鍵に署名し、`keys/query` の `signatures` の行（`e2ee_key_signatures`）を返す。
     * サーバーは端末鍵の JSON を書き換えず、署名を別の行で返す。
     */
    signDevice(store: CryptoStore): JsonObject {
      const ssk = cs.selfSigning;
      const signed = ssk.signDevice(store.deviceKeys());
      ssk.free();
      return { signer_user_id: userId, ...signed } as unknown as JsonObject;
    },
  };
}

type Identity = ReturnType<typeof identity>;

/** `users` のクロス署名鍵を載せた `keys/query` の応答。`me` にだけユーザー署名鍵を返す。 */
function keysQueryWith(
  me: string,
  users: Record<string, Identity>,
  devices: CryptoStore[],
  signatures: JsonObject[],
  tofuPins: Record<string, JsonObject> = {},
): KeysQueryResponse {
  const deviceKeys: Record<string, Record<string, JsonObject>> = {};
  for (const device of devices) {
    (deviceKeys[device.userId] ??= {})[device.deviceId] = device.deviceKeys();
  }
  const query = keysQueryOf(deviceKeys);
  for (const [userId, user] of Object.entries(users)) {
    query.master_keys[userId] = user.master;
    query.self_signing_keys[userId] = user.selfSigning;
    if (userId === me) query.user_signing_keys[userId] = user.userSigning;
  }
  query.signatures = signatures;
  query.tofu_pins = tofuPins;
  return query;
}

/** `sender` から `receiver` にセッション鍵を配り、`keysQuery` で受け取らせて、1 通を復号させる。 */
async function sendAndDecrypt(
  sender: CryptoStore,
  receiver: CryptoStore,
  keysQuery: KeysQueryResponse,
  id = "1",
) {
  const device = recipient(receiver);
  sender.megolm.setDistribution(CHANNEL, 1, [device.device()]);
  await sender.megolm.start(CHANNEL);
  const { messages } = await sender.megolm.share(CHANNEL, device.claimed);
  sender.megolm.markShared(CHANNEL, [{ userId: receiver.userId, deviceId: receiver.deviceId }]);
  expect((await deliver(sender, receiver, messages, id, keysQuery)).roomKeys).toBe(1);
  const encrypted = sender.megolm.encrypt(CHANNEL, outgoing(`m${id}`, "hello"));
  const [received] = await receiver.megolm.decrypt([row(sender, `m${id}`, encrypted)]);
  return received;
}

describe("受信時の検証の手順 4（送信端末を検証済みにする）", () => {
  it("自分の別の端末のメッセージは、解錠済みなら検証済みになる", async () => {
    const me = identity(BOB);
    const other = await openStore(BOB);
    const current = await openStore(BOB);
    await current.store.saveOwnKeys(me.secrets, 1);
    const query = keysQueryWith(BOB, { [BOB]: me }, [other.store], [me.signDevice(other.store)]);

    expect(await sendAndDecrypt(other.store, current.store, query)).toMatchObject({
      status: "decrypted",
      sender: "verified",
    });

    // 解錠していない端末は、自分のマスター鍵を信頼していないので、検証済みにしない。
    const locked = await openStore(BOB);
    expect(await sendAndDecrypt(other.store, locked.store, query, "2")).toMatchObject({
      status: "decrypted",
      sender: "unverified",
    });
    await other.store.close();
    await current.store.close();
    await locked.store.close();
  });

  /** Bob が Alice のマスター鍵を TOFU で信頼した（ピンを置けた）状態を作る。 */
  async function tofuTrusted() {
    const aliceId = identity(ALICE);
    const bobId = identity(BOB);
    const alice = await openStore(ALICE);
    const bob = await openStore(BOB);
    await bob.store.saveOwnKeys(bobId.secrets, 1);
    const snapshot = keysQueryWith(
      BOB,
      { [ALICE]: aliceId, [BOB]: bobId },
      [alice.store],
      [aliceId.signDevice(alice.store)],
    );
    const summary = await bob.store.megolm.evaluate(CHANNEL, [
      {
        ...snapshot,
        channel_id: CHANNEL,
        membership_epoch: 1,
        user_ids: [ALICE, BOB],
      } as unknown as JsonObject,
    ]);
    expect(summary.newPins).toHaveLength(1);
    const pin = summary.newPins[0]!;
    bob.store.trustStore.confirmTofuPin(pin);
    await bob.store.saveTrustStore();
    return { aliceId, bobId, alice, bob, pins: { [ALICE]: pin.pin } };
  }

  it("他のユーザーの、TOFU で信頼した端末のメッセージは検証済みになる", async () => {
    const { aliceId, bobId, alice, bob, pins } = await tofuTrusted();
    const query = keysQueryWith(
      BOB,
      { [ALICE]: aliceId, [BOB]: bobId },
      [alice.store],
      [aliceId.signDevice(alice.store)],
      pins,
    );
    expect(await sendAndDecrypt(alice.store, bob.store, query)).toMatchObject({
      status: "decrypted",
      sender: "verified",
    });
    await alice.store.close();
    await bob.store.close();
  });

  it("初めて見た相手は、ピンを作らず未検証のままにする", async () => {
    const aliceId = identity(ALICE);
    const bobId = identity(BOB);
    const alice = await openStore(ALICE);
    const bob = await openStore(BOB);
    await bob.store.saveOwnKeys(bobId.secrets, 1);
    const query = keysQueryWith(
      BOB,
      { [ALICE]: aliceId, [BOB]: bobId },
      [alice.store],
      [aliceId.signDevice(alice.store)],
    );
    expect(await sendAndDecrypt(alice.store, bob.store, query)).toMatchObject({
      sender: "unverified",
    });
    expect(bob.store.trustStore.record(ALICE)).toBeUndefined();
    await alice.store.close();
    await bob.store.close();
  });

  it.each([
    ["自己署名鍵の署名がない端末", "unsigned"],
    ["偽の自己署名鍵で署名した端末", "forged"],
  ] as const)("%sは未検証のままにする", async (_, kind) => {
    const { aliceId, bobId, alice, bob, pins } = await tofuTrusted();
    const sender = await openStore(ALICE);
    let signatures: JsonObject[] = [];
    let users = { [ALICE]: aliceId, [BOB]: bobId };
    if (kind === "forged") {
      // 攻撃者が自分のマスター鍵で署名した自己署名鍵を Alice のものとして出し、その鍵で端末に署名した。
      const attacker = identity(ALICE);
      signatures = [attacker.signDevice(sender.store)];
      users = { [ALICE]: { ...aliceId, selfSigning: attacker.selfSigning }, [BOB]: bobId };
    }
    const query = keysQueryWith(BOB, users, [sender.store], signatures, pins);
    expect(await sendAndDecrypt(sender.store, bob.store, query)).toMatchObject({
      status: "decrypted",
      sender: "unverified",
    });
    await sender.store.close();
    await alice.store.close();
    await bob.store.close();
  });
});

describe("鍵バックアップからの取り込み（新しい端末での解錠の手順 5）", () => {
  /** `me` の今の版（`GET /api/e2ee/key_backup/versions/current` の応答）。 */
  function currentVersion(me: Identity): JsonObject {
    const backupKey = me.secrets.backupKey;
    const cs = me.secrets.crossSigning;
    const master = cs.master;
    const request = backupKey.createVersionRequest(master, null, 1);
    master.free();
    cs.free();
    backupKey.free();
    return { version: "1", algorithm: request.algorithm, auth_data: request.auth_data, count: 1 };
  }

  /** 端末 A が Alice から受け取った鍵を、鍵バックアップに上げる。上げた行を返す。 */
  async function backedUpOnDeviceA(me: Identity = identity(BOB)) {
    const version = currentVersion(me);
    const alice = await openStore(ALICE);
    const deviceA = await openStore(BOB);
    await deviceA.store.saveOwnKeys(me.secrets, 1);
    expect(
      await sendAndDecrypt(
        alice.store,
        deviceA.store,
        keysQueryOf({
          [ALICE]: { [alice.store.deviceId]: alice.store.deviceKeys() },
        }),
      ),
    ).toMatchObject({ status: "decrypted" });
    const upload = await deviceA.store.prepareBackupUpload(version, 1000);
    if (upload.kind !== "upload") throw new Error(`アップロードできません: ${upload.kind}`);
    await deviceA.store.backupUploaded(upload.uploadId);
    const rows = (upload.body as { sessions: JsonObject[] }).sessions;
    expect(rows).toHaveLength(1);
    // 端末 A が過去に受け取ったメッセージ（新しい端末 B には鍵が届いていない）。
    const encrypted = alice.store.megolm.encrypt(CHANNEL, outgoing("old", "過去のメッセージ"));
    const message = row(alice.store, "old", encrypted);
    await deviceA.store.close();
    return { me, version, alice, rows, message };
  }

  it("端末 A で受け取った鍵を、新しい端末 B で解錠したあとに取り込み、過去のメッセージを読める", async () => {
    const { me, version, alice, rows, message } = await backedUpOnDeviceA();
    const deviceB = await openStore(BOB);
    await deviceB.store.saveOwnKeys(me.secrets, 1);
    expect(await deviceB.store.megolm.decrypt([message])).toEqual([
      { status: "undecryptable", reason: "keyMissing" },
    ]);

    expect(await deviceB.store.beginBackupImport(version)).toEqual({
      kind: "import",
      version: "1",
      after: null,
      pages: 0,
    });
    expect(await deviceB.store.importBackupPage("1", rows, null)).toEqual({
      imported: 1,
      dropped: 0,
    });
    // バックアップ由来のセッションで復号したので、送信者は確かめられない。
    expect(await deviceB.store.megolm.decrypt([message])).toMatchObject([
      { status: "decrypted", body: "過去のメッセージ", sender: "unauthenticated" },
    ]);
    // 同じ版には上げ直さない。取り込み終えた版は読み直さない。
    expect(await deviceB.store.pendingBackupCount()).toBe(0);
    expect(await deviceB.store.prepareBackupUpload(version, 1000)).toEqual({ kind: "none" });
    expect(await deviceB.store.beginBackupImport(version)).toEqual({ kind: "done", version: "1" });
    await deviceB.store.close();
    await alice.store.close();
  });

  it("途中で止まった取り込みは、保存した続きのカーソルから始める", async () => {
    const { me, version, alice, rows } = await backedUpOnDeviceA();
    const deviceB = await openStore(BOB);
    await deviceB.store.saveOwnKeys(me.secrets, 1);
    await deviceB.store.beginBackupImport(version);
    await deviceB.store.importBackupPage("1", rows, "cursor-1");
    expect(await deviceB.store.beginBackupImport(version)).toEqual({
      kind: "import",
      version: "1",
      after: "cursor-1",
      pages: 1,
    });
    await deviceB.store.close();
    await alice.store.close();
  });

  it("空のページだけで終わった版は、取り込み済みにせず、次も始めから読む", async () => {
    const { me, version, alice, rows, message } = await backedUpOnDeviceA();
    const deviceB = await openStore(BOB);
    await deviceB.store.saveOwnKeys(me.secrets, 1);
    await deviceB.store.beginBackupImport(version);
    // サーバーが一時的に（またはわざと）空を返した。
    expect(await deviceB.store.importBackupPage("1", [], null)).toEqual({
      imported: 0,
      dropped: 0,
    });
    expect(await deviceB.store.beginBackupImport(version)).toEqual({
      kind: "import",
      version: "1",
      after: null,
      pages: 0,
    });
    // 空のまま何度読み直しても、ページの上限には届かない。
    for (let i = 0; i < 1001; i += 1) await deviceB.store.importBackupPage("1", [], null);
    // 正しく返るようになったら取り込める。
    expect(await deviceB.store.importBackupPage("1", rows, null)).toEqual({
      imported: 1,
      dropped: 0,
    });
    expect(await deviceB.store.megolm.decrypt([message])).toMatchObject([{ status: "decrypted" }]);
    expect(await deviceB.store.beginBackupImport(version)).toEqual({ kind: "done", version: "1" });
    await deviceB.store.close();
    await alice.store.close();
  });

  it("1 ページの行が多すぎるページと、上限を超えたページは保存しない", async () => {
    const { me, version, alice, rows } = await backedUpOnDeviceA();
    const deviceB = await openStore(BOB);
    await deviceB.store.saveOwnKeys(me.secrets, 1);
    await deviceB.store.beginBackupImport(version);
    const tooMany = Array.from({ length: 1001 }, () => rows[0]!);
    await expect(deviceB.store.importBackupPage("1", tooMany, null)).rejects.toThrow();
    expect(deviceB.store.isBroken).toBe(false);
    for (let i = 0; i < 1000; i += 1) await deviceB.store.importBackupPage("1", [], `c${i}`);
    await expect(deviceB.store.importBackupPage("1", rows, null)).rejects.toThrow();
    await deviceB.store.close();
    await alice.store.close();
  });

  it("壊れた行と、別のセッションの行は取り込まない", async () => {
    const { me, version, alice, rows, message } = await backedUpOnDeviceA();
    const good = rows[0]! as Record<string, unknown> & { session_data: Record<string, string> };
    const other = await backedUpOnDeviceA();
    const otherRow = other.rows[0]! as Record<string, unknown>;
    const broken = [
      // MAC が合わない（暗号文を書き換えた）。
      { ...good, session_data: { ...good.session_data, ciphertext: good.session_data.mac } },
      // 別のセッションの行に、このセッションの暗号文を入れた（行と平文の session_id が違う）。
      { ...good, session_id: otherRow.session_id },
      // 別のチャンネルの行として出した（行と平文の channel_id が違う）。
      { ...good, channel_id: "00000000-0000-4000-8000-0000000000c2" },
      // 別のユーザーのバックアップ鍵で暗号化した行。
      otherRow,
    ] as JsonObject[];
    const deviceB = await openStore(BOB);
    await deviceB.store.saveOwnKeys(me.secrets, 1);
    await deviceB.store.beginBackupImport(version);
    expect(await deviceB.store.importBackupPage("1", broken, null)).toEqual({
      imported: 0,
      dropped: 4,
    });
    expect(await deviceB.store.megolm.decrypt([message])).toEqual([
      { status: "undecryptable", reason: "keyMissing" },
    ]);
    await deviceB.store.close();
    await alice.store.close();
    await other.alice.store.close();
  });

  it("ページの途中で読み取りに失敗したら、開き直すまで使わず、開き直して試し直すとすべて保存する", async () => {
    class FlakyBackend extends MemoryCryptoBackend {
      failOnRead = 0;
      override async readInboundSession(key: string): Promise<string | null> {
        this.failOnRead -= 1;
        if (this.failOnRead === 0) throw new Error("I/O error");
        return super.readInboundSession(key);
      }
    }
    const first = await backedUpOnDeviceA();
    const second = await backedUpOnDeviceA(first.me);
    const rows = [...first.rows, ...second.rows];
    const backend = new FlakyBackend();
    const keys = memoryKeys();
    const openB = async () =>
      (
        await CryptoStore.open({
          backend,
          name: "zoo-crypto-device-b.sqlite",
          userId: BOB,
          keys,
          reset: async () => backend,
        })
      ).store;
    const deviceB = await openB();
    await deviceB.saveOwnKeys(first.me.secrets, 1);
    await deviceB.beginBackupImport(first.version);
    // 1 行目はメモリーに取り込み、2 行目の読み取りで失敗する。
    backend.failOnRead = 2;
    await expect(deviceB.importBackupPage("1", rows, null)).rejects.toThrow("I/O error");
    expect(deviceB.isBroken).toBe(true);
    expect(backend.inboundSessions.size).toBe(0);
    await deviceB.close();

    const reopened = await openB();
    expect(await reopened.beginBackupImport(first.version)).toMatchObject({
      after: null,
      pages: 0,
    });
    expect(await reopened.importBackupPage("1", rows, null)).toEqual({
      imported: 2,
      dropped: 0,
    });
    expect(backend.inboundSessions.size).toBe(2);
    await reopened.close();
    await first.alice.store.close();
    await second.alice.store.close();
  });

  it("署名のない版や、別のバックアップ鍵の版からは取り込まない", async () => {
    const { me, alice } = await backedUpOnDeviceA();
    const deviceB = await openStore(BOB);
    await deviceB.store.saveOwnKeys(me.secrets, 1);
    const stranger = currentVersion(identity(BOB));
    expect(await deviceB.store.beginBackupImport(stranger)).toMatchObject({ kind: "rejected" });
    expect(await deviceB.store.beginBackupImport(null)).toEqual({ kind: "none" });
    // 受け入れていない版のページは保存しない。
    await expect(deviceB.store.importBackupPage("1", [], null)).rejects.toThrow();
    // 解錠していない端末は取り込まない。
    const locked = await openStore(BOB);
    expect(await locked.store.beginBackupImport(currentVersion(me))).toEqual({ kind: "none" });
    await deviceB.store.close();
    await locked.store.close();
    await alice.store.close();
  });
});
