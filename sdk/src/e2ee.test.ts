// Needs the wasm package: crates/zoo-crypto-wasm/build.sh (or ZOO_CRYPTO_WASM_DIR).
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { test } from "node:test";

import { ZooApp } from "./index.ts";
import { E2eeSendError, ZooE2ee, loadZooCrypto, memoryStore, type E2eeStore } from "./e2ee.ts";
import { FakeZoo, HumanDevice } from "./testing/fakeZoo.ts";

const crypto = await loadZooCrypto();
const storageKey = new Uint8Array(32).fill(3);

function setup(scopes?: string[]) {
  const zoo = new FakeZoo(randomUUID(), scopes);
  const app = new ZooApp({ baseUrl: "https://zoo.example", credentials: { access_token: "a", refresh_token: "r" }, fetch: zoo.fetch });
  return { zoo, app };
}

async function openBot(zoo: FakeZoo, app: ZooApp, store: E2eeStore = memoryStore()) {
  return ZooE2ee.open({ app, crypto, store, storageKey, sleep: async () => {} });
}

test("sets up a cross-signed device once and restores it from the store", async () => {
  const { zoo, app } = setup();
  const store = memoryStore();
  const bot = await openBot(zoo, app, store);
  const devices = [...zoo.devices.get(zoo.botUserId)!.keys()];
  assert.deepEqual(devices, [bot.deviceId]);
  const keys = zoo.crossSigning.get(zoo.botUserId)!;
  assert.ok(keys.master && keys.self_signing && keys.user_signing);
  const selfSigning = Object.values(keys.self_signing.keys as Record<string, string>)[0];
  assert.ok(zoo.signatures.some((s) => s.signer_user_id === zoo.botUserId && s.target_key_id === bot.deviceId && s.signer_key_id === `ed25519:${selfSigning}`));
  assert.equal(zoo.oneTimeKeys.get(`${zoo.botUserId}|${bot.deviceId}`)!.size, 50);
  // The store holds pickles only: no key material in the clear.
  assert.doesNotMatch(store.data!, new RegExp(selfSigning!.replace(/[+/]/g, ".")));
  bot.close();

  const again = await openBot(zoo, app, store);
  assert.equal(again.deviceId, bot.deviceId);
  assert.equal(zoo.devices.get(zoo.botUserId)!.size, 1);
  again.close();
  await assert.rejects(ZooE2ee.open({ app, crypto, store, storageKey: new Uint8Array(32).fill(9) }));
});

test("a lost store replaces the device and the cross-signing keys", async () => {
  const { zoo, app } = setup();
  const first = await openBot(zoo, app);
  const master = JSON.stringify(zoo.crossSigning.get(zoo.botUserId)!.master);
  first.close();
  const second = await openBot(zoo, app);
  assert.notEqual(second.deviceId, first.deviceId);
  assert.deepEqual([...zoo.devices.get(zoo.botUserId)!.keys()], [second.deviceId]);
  assert.notEqual(JSON.stringify(zoo.crossSigning.get(zoo.botUserId)!.master), master);
  second.close();
});

test("refuses delegated tokens and tokens without the e2ee:device scope", async () => {
  const { zoo, app } = setup(["messages:read", "messages:write"]);
  await assert.rejects(openBot(zoo, app), /e2ee:device/);
});

test("decrypts a member's message and the member reads the encrypted reply", async () => {
  const { zoo, app } = setup();
  const bot = await openBot(zoo, app);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);

  // Alice's client trusts the bot's master key on first use and shares the key with its signed device.
  const sent = alice.send(channel, "hello bot", { mentions: [zoo.botUserId] });
  assert.ok(sent.devices.includes(`${zoo.botUserId}|${bot.deviceId}`));
  const message = await bot.decryptEvent(zoo.botEvents[0]!);
  assert.deepEqual(
    { body: message?.body, userId: message?.userId, verified: message?.senderVerified, mentions: message?.mentions.user_ids },
    { body: "hello bot", userId: alice.userId, verified: true, mentions: [zoo.botUserId] },
  );
  assert.equal(zoo.toDevice.length, 0, "processed to-device messages are acknowledged");

  const replyId = await bot.sendMessage(channel, "hello alice", { parentId: sent.messageId });
  const reply = zoo.messages.find((m) => m.id === replyId)!;
  assert.equal(reply.user_id, zoo.botUserId);
  assert.doesNotMatch(JSON.stringify(reply), /hello alice/);
  assert.equal(alice.receiveKeys(), 1);
  const decrypted = alice.decrypt(reply);
  assert.equal(decrypted.payload.content.body, "hello alice");
  assert.equal(decrypted.payload.parent_id, sent.messageId);

  // The same session is reused, and its key is not shared twice.
  await bot.sendMessage(channel, "again");
  assert.equal(zoo.toDevice.length, 0);
  assert.equal(alice.decrypt(zoo.messages.at(-1)!).payload.content.body, "again");
  bot.close();
});

test("restored bots keep reading sessions they received before a restart", async () => {
  const { zoo, app } = setup();
  const store = memoryStore();
  const bot = await openBot(zoo, app, store);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  alice.send(channel, "one");
  assert.equal((await bot.decryptEvent(zoo.botEvents[0]!))?.body, "one");
  bot.close();
  const restored = await openBot(zoo, app, store);
  alice.send(channel, "two");
  assert.equal((await restored.decryptEvent(zoo.botEvents[1]!))?.body, "two");
  restored.close();
});

test("shares keys only with verified devices, and stops when a member's master key changes", async () => {
  const { zoo, app } = setup();
  const bot = await openBot(zoo, app);
  const alice = new HumanDevice(crypto, zoo);
  const mallory = new HumanDevice(crypto, zoo, { signDevice: false });
  const channel = zoo.channel([alice.userId, mallory.userId, zoo.botUserId]);

  await bot.sendMessage(channel, "first");
  const targets = zoo.toDevice.map((m) => m.recipient_user_id);
  assert.deepEqual(targets, [alice.userId], "the unsigned device gets nothing");
  assert.equal(alice.receiveKeys(), 1);

  // Alice's master key changes (for example an attacker re-registers her keys): the bot stops sharing.
  alice.resetCrossSigning();
  await bot.sendMessage(channel, "second");
  assert.equal(zoo.toDevice.filter((m) => m.recipient_user_id === alice.userId).length, 0);
  bot.close();
});

test("messages from unverified devices are decrypted but marked unverified", async () => {
  const { zoo, app } = setup();
  const bot = await openBot(zoo, app);
  const mallory = new HumanDevice(crypto, zoo, { signDevice: false });
  const channel = zoo.channel([mallory.userId, zoo.botUserId]);
  mallory.send(channel, "trust me");
  const message = await bot.decryptEvent(zoo.botEvents[0]!);
  assert.equal(message?.body, "trust me");
  assert.equal(message?.senderVerified, false);
  bot.close();
});

test("a bot that was not allowed into the channel gets no usable key", async () => {
  const { zoo, app } = setup();
  const bot = await openBot(zoo, app);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId], false);
  // Even if a member's client shared a key, the bot cannot read the channel's snapshot and drops it.
  alice.send(channel, "secret");
  const event = { id: randomUUID(), version: 1 as const, type: "message.created", actor_id: alice.userId, channel_id: channel, data: { ...zoo.messages[0]!, encrypted: true } };
  await assert.rejects(bot.decryptEvent(event), (error: { code?: string }) => error.code === "MissingRoomKey");
  bot.close();
});

test("re-shares with a new session when the membership epoch changes during a send", async () => {
  const { zoo, app: _ } = setup();
  let bumped = false;
  const fetcher: typeof fetch = async (input, init) => {
    if (!bumped && String(input).endsWith("/api/apps/v1/messages")) {
      bumped = true;
      zoo.bumpEpoch(channel);
    }
    return zoo.fetch(input, init);
  };
  const app = new ZooApp({ baseUrl: "https://zoo.example", credentials: { access_token: "a", refresh_token: "r" }, fetch: fetcher });
  const bot = await openBot(zoo, app);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  const id = await bot.sendMessage(channel, "after the change");
  assert.equal(zoo.messages.length, 1);
  assert.equal(zoo.messages[0]!.epoch, 2);
  assert.equal(alice.receiveKeys(), 2, "a key for the first session and one for the new epoch");
  assert.equal(alice.decrypt(zoo.messages[0]!).payload.message_id, id);

  // An epoch that never settles gives up instead of looping.
  const restless = new ZooApp({
    baseUrl: "https://zoo.example",
    credentials: { access_token: "a", refresh_token: "r" },
    fetch: async (input, init) => {
      if (String(input).endsWith("/api/apps/v1/messages")) zoo.bumpEpoch(channel);
      return zoo.fetch(input, init);
    },
  });
  const other = await ZooE2ee.open({ app: restless, crypto, store: memoryStore(), storageKey, sleep: async () => {} });
  await assert.rejects(other.sendMessage(channel, "never"), E2eeSendError);
  other.close();
  bot.close();
});

/** A ZooApp whose requests matching `fail` get a 503 while `failures()` returns true. */
function flakyApp(zoo: FakeZoo, fail: (url: string, method: string) => boolean) {
  return new ZooApp({
    baseUrl: "https://zoo.example",
    credentials: { access_token: "a", refresh_token: "r" },
    fetch: async (input, init) => (fail(String(input), init?.method ?? "GET") ? Response.json({ error: "unavailable" }, { status: 503 }) : zoo.fetch(input, init)),
  });
}

test("keeps a received session key when the channel's snapshot fails after decryption", async () => {
  const zoo = new FakeZoo(randomUUID());
  let failures = 0;
  const app = flakyApp(zoo, (url) => url.includes("/recipients") && failures-- > 0);
  const bot = await openBot(zoo, app);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);

  // A short outage: retried, and the sender is still verified.
  failures = 2;
  alice.send(channel, "one");
  const first = await bot.decryptEvent(zoo.botEvents[0]!);
  assert.deepEqual([first?.body, first?.senderVerified], ["one", true]);

  // A longer outage: the key is kept (never lost to a decrypt-once Olm message), as unverified.
  zoo.bumpEpoch(channel);
  failures = 100;
  alice.send(channel, "two");
  const second = await bot.decryptEvent(zoo.botEvents[1]!);
  assert.deepEqual([second?.body, second?.senderVerified], ["two", false]);
  assert.equal(zoo.toDevice.length, 0);
  bot.close();
});

test("leaves a to-device message queued when a request before decryption fails", async () => {
  const zoo = new FakeZoo(randomUUID());
  let down = false;
  const app = flakyApp(zoo, (url) => down && url.endsWith("/e2ee/keys/query"));
  const bot = await openBot(zoo, app);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  alice.send(channel, "queued");
  down = true;
  await assert.rejects(bot.syncToDevice());
  assert.equal(zoo.toDevice.length, 1, "not acknowledged");
  down = false;
  assert.equal((await bot.decryptEvent(zoo.botEvents[0]!))?.body, "queued");
  bot.close();
});

test("stops without acknowledging when the store cannot be saved, and a reopened device reads the key", async () => {
  const zoo = new FakeZoo(randomUUID());
  const app = new ZooApp({ baseUrl: "https://zoo.example", credentials: { access_token: "a", refresh_token: "r" }, fetch: zoo.fetch });
  const backing = memoryStore();
  let readOnly = false;
  const store: E2eeStore = {
    load: () => backing.load(),
    save: async (data) => {
      if (readOnly) throw new Error("disk full");
      await backing.save(data);
    },
  };
  const bot = await openBot(zoo, app, store);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  alice.send(channel, "after the disk is fixed");
  readOnly = true;
  await assert.rejects(bot.syncToDevice(), /disk full/);
  assert.equal(zoo.toDevice.length, 1, "not acknowledged");
  await assert.rejects(bot.syncToDevice(), /open the device again/);
  bot.close();
  readOnly = false;
  const reopened = await openBot(zoo, app, store);
  assert.equal((await reopened.decryptEvent(zoo.botEvents[0]!))?.body, "after the disk is fixed");
  reopened.close();
});

test("a self-signing key the sender's master key never signed does not make the sender verified", async () => {
  const { zoo, app } = setup();
  const bot = await openBot(zoo, app);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  alice.send(channel, "genuine");
  assert.equal((await bot.decryptEvent(zoo.botEvents[0]!))?.senderVerified, true, "the bot pinned Alice's master key");

  // The server keeps Alice's pinned master key but swaps in its own self-signing key, signed by
  // another master key, and signs her device with it.
  const forged = crypto.OwnSecrets.generate(alice.userId).crossSigning;
  zoo.crossSigning.get(alice.userId)!.self_signing = forged.uploadJson().self_signing_key as Record<string, unknown> as never;
  zoo.signatures.push({ ...forged.selfSigning.signDevice(alice.account.deviceKeys()), signer_user_id: alice.userId });
  zoo.bumpEpoch(channel);
  alice.send(channel, "forged");
  const message = await bot.decryptEvent(zoo.botEvents[1]!);
  assert.equal(message?.body, "forged");
  assert.equal(message?.senderVerified, false);
  bot.close();
});

test("fileStore writes the state atomically and reads it back", async () => {
  const { mkdtemp, readdir, rm, stat } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { fileStore } = await import("./e2ee.ts");
  const dir = await mkdtemp(join(tmpdir(), "zoo-e2ee-"));
  try {
    const store = fileStore(join(dir, "store.json"));
    assert.equal(await store.load(), null);
    await store.save('{"a":1}');
    await store.save('{"a":2}');
    assert.equal(await store.load(), '{"a":2}');
    assert.deepEqual(await readdir(dir), ["store.json"], "no temporary file is left");
    assert.equal((await stat(join(dir, "store.json"))).mode & 0o777, 0o600);
  } finally {
    await rm(dir, { recursive: true });
  }
});

test("one-time keys made while replenishing survive a restart", async () => {
  const { zoo, app } = setup();
  const store = memoryStore();
  const bot = await openBot(zoo, app, store);
  // The server ran out of the bot's one-time keys (claimed by other devices).
  zoo.oneTimeKeys.get(`${zoo.botUserId}|${bot.deviceId}`)!.clear();
  zoo.fallbackKeys.delete(`${zoo.botUserId}|${bot.deviceId}`);
  await bot.replenishKeys();
  bot.close();
  const restored = await openBot(zoo, app, store);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  alice.send(channel, "uses a replenished key");
  assert.equal((await restored.decryptEvent(zoo.botEvents[0]!))?.body, "uses a replenished key");
  restored.close();
});

test("keys replenished while a to-device message is being verified are kept", async () => {
  const zoo = new FakeZoo(randomUUID());
  let during: (() => Promise<void>) | null = null;
  const app = new ZooApp({
    baseUrl: "https://zoo.example",
    credentials: { access_token: "a", refresh_token: "r" },
    fetch: async (input, init) => {
      // Replenish while the bot reads the channel's snapshot for a received key.
      if (during && String(input).includes("/recipients")) {
        const run = during;
        during = null;
        await run();
      }
      return zoo.fetch(input, init);
    },
  });
  const store = memoryStore();
  const bot = await openBot(zoo, app, store);
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  alice.send(channel, "first");
  during = async () => {
    zoo.oneTimeKeys.get(`${zoo.botUserId}|${bot.deviceId}`)!.clear();
    zoo.fallbackKeys.delete(`${zoo.botUserId}|${bot.deviceId}`);
    await bot.replenishKeys();
  };
  assert.equal((await bot.decryptEvent(zoo.botEvents[0]!))?.body, "first");
  bot.close();
  const restored = await openBot(zoo, app, store);
  const bob = new HumanDevice(crypto, zoo);
  const other = zoo.channel([bob.userId, zoo.botUserId]);
  bob.send(other, "uses a key made during the first sync");
  assert.equal((await restored.decryptEvent(zoo.botEvents[1]!))?.body, "uses a key made during the first sync");
  restored.close();
});
