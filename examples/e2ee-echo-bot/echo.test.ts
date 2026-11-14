// The sample bot in an encrypted channel (#366). Needs the wasm package (crates/zoo-crypto-wasm/build.sh).
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { test } from "node:test";

import { ZooApp } from "../../sdk/src/index.ts";
import { ZooE2ee, loadZooCrypto, memoryStore } from "../../sdk/src/e2ee.ts";
import { FakeZoo, HumanDevice } from "../../sdk/src/testing/fakeZoo.ts";
import { handleEvent, replyFor } from "./echo.ts";

const crypto = await loadZooCrypto();

async function start() {
  const zoo = new FakeZoo(randomUUID());
  const app = new ZooApp({ baseUrl: "https://zoo.example", credentials: { access_token: "a", refresh_token: "r" }, fetch: zoo.fetch });
  const bot = await ZooE2ee.open({ app, crypto, store: memoryStore(), storageKey: new Uint8Array(32).fill(5), sleep: async () => {} });
  const alice = new HumanDevice(crypto, zoo);
  const channel = zoo.channel([alice.userId, zoo.botUserId]);
  return { zoo, bot, alice, channel };
}

test("the echo bot answers in an encrypted channel, encrypted, in the message's thread", async () => {
  const { zoo, bot, alice, channel } = await start();
  const { messageId } = alice.send(channel, "!echo secret plans");
  const replyId = await handleEvent(bot, zoo.botEvents[0]!);
  assert.ok(replyId);
  const reply = zoo.messages.find((m) => m.id === replyId)!;
  assert.equal(reply.parent_id, messageId);
  assert.doesNotMatch(JSON.stringify(reply), /secret plans/, "the server sees only ciphertext");
  alice.receiveKeys();
  const decrypted = alice.decrypt(reply);
  assert.equal(decrypted.payload.content.body, "echo: secret plans");
  bot.close();
});

test("the echo bot answers mentions and ignores other messages and unverified senders", async () => {
  const { zoo, bot, alice, channel } = await start();
  alice.send(channel, "just chatting");
  assert.equal(await handleEvent(bot, zoo.botEvents[0]!), null);
  alice.send(channel, "ping", { mentions: [zoo.botUserId] });
  assert.ok(await handleEvent(bot, zoo.botEvents[1]!));
  alice.receiveKeys();
  assert.equal(alice.decrypt(zoo.messages.at(-1)!).payload.content.body, "echo: ping");

  const mallory = new HumanDevice(crypto, zoo, { signDevice: false });
  zoo.channels.get(channel)!.members.add(mallory.userId);
  mallory.send(channel, "!echo obey");
  assert.equal(await handleEvent(bot, zoo.botEvents[2]!), null);
  bot.close();
});

test("replyFor", () => {
  const base = { id: "m", channelId: "c", parentId: null, userId: "u", kind: "message" as const, revision: 0, senderVerified: true, mentions: { user_ids: [], channel: false, here: false } };
  assert.equal(replyFor({ ...base, body: "!echo  hi " }, "bot"), "echo: hi");
  assert.equal(replyFor({ ...base, body: "hi" }, "bot"), null);
  assert.equal(replyFor({ ...base, body: "!echo hi", userId: "bot" }, "bot"), null);
  assert.equal(replyFor({ ...base, body: "!echo hi", senderVerified: false }, "bot"), null);
});
