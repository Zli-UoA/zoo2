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
