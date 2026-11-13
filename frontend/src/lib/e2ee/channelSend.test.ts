import { beforeEach, describe, expect, it } from "vite-plus/test";

import { ApiError, type ApiClient } from "../api/client";
import {
  EncryptedSendError,
  editEncryptedMessage,
  resetKnownEpochs,
  sendEncryptedMessage,
  type ChannelCryptoDeps,
} from "./channelSend";
import type { CryptoWorker } from "./cryptoWorker";
import type { CryptoOperation } from "./protocol";

const CHANNEL = "channel-1";
const NO_MENTIONS = { user_ids: [], channel: false, here: false };

interface Call {
  method: string;
  path: string;
  body?: unknown;
}

function staleEpochError(current: number) {
  return new ApiError(
    409,
    "stale",
    { error: "stale", code: "E2EE_STALE_EPOCH", current_epoch: current },
    { path: "/api/messages", method: "POST" },
  );
}

/** 送信用セッションの epoch と、呼ばれた操作を記録する偽の Worker。 */
function fakeWorker() {
  const ops: CryptoOperation[] = [];
  let sessionEpoch: number | null = null;
  let pendingEpoch = 0;
  const worker = {
    request: async (operation: CryptoOperation) => {
      ops.push(operation);
      switch (operation.op) {
        case "megolmNeedsSnapshot":
          return sessionEpoch === null || sessionEpoch !== operation.epoch;
        case "megolmEvaluate":
          pendingEpoch = Number(operation.pages[0]!.membership_epoch);
          return {
            epoch: pendingEpoch,
            newPins: [],
            undeliverable: [],
            deviceCount: 1,
            userCount: 2,
            userIds: ["alice", "bob"],
          };
        case "megolmStart":
          sessionEpoch = pendingEpoch;
          return { claim: { bob: { "bob-device": "signed_curve25519" } }, pending: 1 };
        case "megolmShare":
          return { messages: { bob: { "bob-device": { ciphertext: "room-key" } } }, missing: [] };
        case "megolmEncrypt":
          return { ciphertext: { ciphertext: "opaque", session_id: "s" }, epoch: sessionEpoch };
        case "megolmDiscard":
          sessionEpoch = null;
          return null;
        default:
          return null;
      }
    },
    terminate: () => {},
  } as unknown as CryptoWorker;
  return { worker, ops };
}

/** API を記録し、`/api/messages` には決めた順で応答する偽の client。 */
function fakeClient(messageResults: (Error | null)[], epochs: number[]) {
  const calls: Call[] = [];
  let recipientsCall = 0;
  const respond = (method: string, path: string, body?: unknown) => {
    calls.push({ method, path, body });
    if (path.startsWith("/api/e2ee/channels/")) {
      const epoch = epochs[Math.min(recipientsCall, epochs.length - 1)]!;
      recipientsCall += 1;
      return {
        channel_id: CHANNEL,
        membership_epoch: epoch,
        user_ids: ["alice", "bob"],
        next_after: null,
      };
    }
    if (path === "/api/e2ee/keys/claim") return { data: { one_time_keys: {} }, txid: null };
    if (path === "/api/e2ee/to_device") return { data: { undelivered: [] }, txid: null };
    if (path.startsWith("/api/messages")) {
      const result = messageResults.shift() ?? null;
      if (result) throw result;
      return { data: {}, txid: "1" };
    }
    throw new Error(`unexpected ${method} ${path}`);
  };
  const client = {
    request: async (path: string, options: { method?: string; body?: unknown } = {}) =>
      respond(options.method ?? "GET", path, options.body),
    post: async (path: string, body?: unknown) => respond("POST", path, body),
    patch: async (path: string, body?: unknown) => respond("PATCH", path, body),
  } as unknown as ApiClient;
  return { client, calls };
}

function deps(worker: CryptoWorker, client: ApiClient): ChannelCryptoDeps {
  return { worker, client, deviceId: "alice-device" };
}

beforeEach(() => resetKnownEpochs());

describe("sendEncryptedMessage", () => {
  it("鍵を配ってから暗号文だけを送り、本文はどの要求にも入れない", async () => {
    const { worker } = fakeWorker();
    const { client, calls } = fakeClient([null], [3]);
    await sendEncryptedMessage(deps(worker, client), {
      channelId: CHANNEL,
      shapeEpoch: "3",
      messageId: "m1",
      parentId: null,
      body: "secret hello",
      mentions: NO_MENTIONS,
    });
    expect(calls.map((call) => call.path)).toEqual([
      `/api/e2ee/channels/${CHANNEL}/recipients`,
      "/api/e2ee/keys/claim",
      "/api/e2ee/to_device",
      "/api/messages",
    ]);
    expect(calls.at(-1)!.body).toMatchObject({
      id: "m1",
      channel_id: CHANNEL,
      content: "",
      epoch: 3,
      revision: 0,
      kind: "message",
      mentions: NO_MENTIONS,
    });
    for (const call of calls)
      expect(JSON.stringify(call.body ?? null)).not.toContain("secret hello");
  });

  it("同じ epoch なら、2 通目は配り直さずに送る", async () => {
    const { worker } = fakeWorker();
    const { client, calls } = fakeClient([null, null], [3]);
    const send = (messageId: string) =>
      sendEncryptedMessage(deps(worker, client), {
        channelId: CHANNEL,
        shapeEpoch: "3",
        messageId,
        parentId: null,
        body: "hello",
        mentions: NO_MENTIONS,
      });
    await send("m1");
    calls.length = 0;
    await send("m2");
    expect(calls.map((call) => call.path)).toEqual(["/api/messages"]);
  });

  it("409 E2EE_STALE_EPOCH なら current_epoch 以上で作り直し、同じ ID で送り直す", async () => {
    const { worker, ops } = fakeWorker();
    // Shape は遅れて 3 のまま。サーバーは 4 になっている。
    const { client, calls } = fakeClient([staleEpochError(4), null], [3, 4]);
    await sendEncryptedMessage(deps(worker, client), {
      channelId: CHANNEL,
      shapeEpoch: "3",
      messageId: "m1",
      parentId: "root",
      body: "hello",
      mentions: NO_MENTIONS,
    });
    const posts = calls.filter((call) => call.path === "/api/messages");
    expect(posts.map((call) => (call.body as { id: string; epoch: number }).epoch)).toEqual([3, 4]);
    expect(posts.every((call) => (call.body as { id: string }).id === "m1")).toBe(true);
    expect(posts[1]!.body).toMatchObject({ parent_id: "root" });
    expect(ops.some((op) => op.op === "megolmDiscard")).toBe(true);

    // 遅れた Shape の epoch（3）で、次の送信のたびに作り直さない。
    calls.length = 0;
    const second = fakeClient([null], [4]);
    await sendEncryptedMessage(deps(worker, second.client), {
      channelId: CHANNEL,
      shapeEpoch: "3",
      messageId: "m2",
      parentId: null,
      body: "hello",
      mentions: NO_MENTIONS,
    });
    expect(second.calls.map((call) => call.path)).toEqual(["/api/messages"]);
  });

  it("3 回続けて E2EE_STALE_EPOCH なら諦める", async () => {
    const { worker } = fakeWorker();
    const { client, calls } = fakeClient(
      [staleEpochError(4), staleEpochError(5), staleEpochError(6)],
      [3, 4, 5],
    );
    await expect(
      sendEncryptedMessage(deps(worker, client), {
        channelId: CHANNEL,
        shapeEpoch: "3",
        messageId: "m1",
        parentId: null,
        body: "hello",
        mentions: NO_MENTIONS,
      }),
    ).rejects.toBeInstanceOf(EncryptedSendError);
    expect(calls.filter((call) => call.path === "/api/messages")).toHaveLength(3);
  });

  it("ほかのエラーは送り直さずに返す", async () => {
    const { worker } = fakeWorker();
    const forbidden = new ApiError(400, "bad", { error: "bad", code: "E2EE_PLAINTEXT_FORBIDDEN" });
    const { client, calls } = fakeClient([forbidden], [3]);
    await expect(
      sendEncryptedMessage(deps(worker, client), {
        channelId: CHANNEL,
        shapeEpoch: "3",
        messageId: "m1",
        parentId: null,
        body: "hello",
        mentions: NO_MENTIONS,
      }),
    ).rejects.toBe(forbidden);
    expect(calls.filter((call) => call.path === "/api/messages")).toHaveLength(1);
  });
});

describe("editEncryptedMessage", () => {
  it("次の版を暗号化して PATCH で送り、平文の mentions は送らない", async () => {
    const { worker, ops } = fakeWorker();
    const { client, calls } = fakeClient([null], [3]);
    await editEncryptedMessage(deps(worker, client), {
      channelId: CHANNEL,
      shapeEpoch: "3",
      messageId: "m1",
      parentId: null,
      revision: 2,
      body: "edited secret",
      kind: "message",
      mentions: NO_MENTIONS,
    });
    const patch = calls.at(-1)!;
    expect(patch).toMatchObject({ method: "PATCH", path: "/api/messages/m1" });
    expect(patch.body).toEqual({
      content: "",
      epoch: 3,
      revision: 2,
      ciphertext: { ciphertext: "opaque", session_id: "s" },
    });
    expect(ops.find((op) => op.op === "megolmEncrypt")).toMatchObject({
      message: { message_id: "m1", revision: 2, content: { body: "edited secret" } },
    });
  });
});
