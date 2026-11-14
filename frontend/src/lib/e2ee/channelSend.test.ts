import { beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { ApiError, type ApiClient } from "../api/client";
import {
  EncryptedSendError,
  editAlreadyApplied,
  editEncryptedMessage,
  resetKnownEpochs,
  sendEncryptedMessage,
  type ChannelCryptoDeps,
} from "./channelSend";
import type { CryptoWorker } from "./cryptoWorker";
import type { CryptoOperation } from "./protocol";
import type { RecipientSummary } from "./store/megolm";

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
function fakeWorker(options: { pendingPins?: number } = {}) {
  const ops: CryptoOperation[] = [];
  // ピンを置くまで配布先に入らない、初めて見た相手の数。
  let pendingPins = options.pendingPins ?? 0;
  let previousUserIds: string[] | null = null;
  let sessionEpoch: number | null = null;
  let pendingEpoch = 0;
  let shared = false;
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
            newPins: Array.from({ length: pendingPins }, (_, index) => ({
              target_user_id: `new-${index}`,
              master_key: "master",
              pin: { pin: index },
            })),
            undeliverable: [],
            deviceCount: 1 + (options.pendingPins ?? 0) - pendingPins,
            userCount: 2,
            userIds: ["alice", "bob"],
          };
        case "megolmStart":
          if (sessionEpoch !== pendingEpoch) shared = false;
          sessionEpoch = pendingEpoch;
          return shared
            ? { claim: {}, pending: 0 }
            : { claim: { bob: { "bob-device": "signed_curve25519" } }, pending: 1 };
        case "megolmSettlePins":
          pendingPins -= operation.confirmed.length;
          return null;
        case "megolmNoteRecipients": {
          const added = previousUserIds
            ? operation.userIds.filter((id) => !previousUserIds!.includes(id))
            : [];
          previousUserIds = operation.userIds;
          return added;
        }
        case "megolmMarkShared":
          if (operation.devices.length > 0) shared = true;
          return null;
        case "megolmShare":
          return { messages: { bob: { "bob-device": { ciphertext: "room-key" } } }, missing: [] };
        case "megolmEncrypt":
          if (sessionEpoch === null)
            throw Object.assign(new Error("no"), { code: "NoOutboundSession" });
          return { ciphertext: { ciphertext: "opaque", session_id: "s" }, epoch: sessionEpoch };
        case "megolmDiscard":
          sessionEpoch = null;
          shared = false;
          return null;
        default:
          return null;
      }
    },
    terminate: () => {},
    /** テスト用：配った端末の記録を消す（`undelivered` で配り終えていない状態）。 */
    markUnshared: () => {
      shared = false;
    },
  } as unknown as CryptoWorker;
  return { worker, ops };
}

/** API を記録し、`/api/messages` には決めた順で応答する偽の client。 */
function fakeClient(
  messageResults: (Error | null)[],
  epochs: number[],
  toDeviceResults: (Error | null)[] = [],
) {
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
    if (path === "/api/e2ee/tofu_pins") return { data: null, txid: null };
    if (path === "/api/e2ee/to_device") {
      const result = toDeviceResults.shift() ?? null;
      if (result) throw result;
      return { data: { undelivered: [] }, txid: null };
    }
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
    let sessionsCreated = 0;
    const send = (messageId: string) =>
      sendEncryptedMessage(
        { ...deps(worker, client), onSessionCreated: () => (sessionsCreated += 1) },
        {
          channelId: CHANNEL,
          shapeEpoch: "3",
          messageId,
          parentId: null,
          body: "hello",
          mentions: NO_MENTIONS,
        },
      );
    await send("m1");
    calls.length = 0;
    await send("m2");
    expect(calls.map((call) => call.path)).toEqual(["/api/messages"]);
    // 新しい送信用セッションを作ったときだけ、鍵バックアップへの送信を頼む。
    expect(sessionsCreated).toBe(1);
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
    // 判定をやり直し、新しい epoch のセッションを作った。
    expect(ops.filter((op) => op.op === "megolmEvaluate")).toHaveLength(2);

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

  it("続けて送っても、スナップショットの取得と鍵の配布は 1 回にまとまる", async () => {
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
    await Promise.all([send("m1"), send("m2")]);
    expect(calls.filter((call) => call.path.endsWith("/recipients"))).toHaveLength(1);
    expect(calls.filter((call) => call.path === "/api/e2ee/to_device")).toHaveLength(1);
    expect(calls.filter((call) => call.path === "/api/messages")).toHaveLength(2);
  });

  it("鍵の配布が途中で失敗したら、次の送信の前に配り直す", async () => {
    const { worker } = fakeWorker();
    const unavailable = new ApiError(503, "unavailable", null, {
      path: "/api/e2ee/to_device",
      method: "POST",
    });
    const { client, calls } = fakeClient([null], [3], [unavailable]);
    const send = (messageId: string) =>
      sendEncryptedMessage(deps(worker, client), {
        channelId: CHANNEL,
        shapeEpoch: "3",
        messageId,
        parentId: null,
        body: "hello",
        mentions: NO_MENTIONS,
      });
    // 1 回目は配布で失敗し、メッセージは送らない（本文は入力欄に戻る）。
    await expect(send("m1")).rejects.toBe(unavailable);
    expect(calls.filter((call) => call.path === "/api/messages")).toHaveLength(0);
    calls.length = 0;
    // 配布の途中で失敗したセッションは捨てるので、2 回目は判定をやり直し、配ってから送る。
    await send("m1");
    expect(calls.map((call) => call.path)).toEqual([
      `/api/e2ee/channels/${CHANNEL}/recipients`,
      "/api/e2ee/keys/claim",
      "/api/e2ee/to_device",
      "/api/messages",
    ]);
  });

  it("判定をやり直さないときも、配り終えていない端末に配ってから送る", async () => {
    const { worker, ops } = fakeWorker();
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
    // to_device の undelivered などで配り終えていない端末が残った状態にする。
    ops.length = 0;
    calls.length = 0;
    (worker as unknown as { markUnshared: () => void }).markUnshared();
    await send("m2");
    expect(ops.filter((op) => op.op === "megolmNeedsSnapshot")).toHaveLength(1);
    expect(calls.map((call) => call.path)).toEqual([
      "/api/e2ee/keys/claim",
      "/api/e2ee/to_device",
      "/api/messages",
    ]);
  });

  it("409 での作り直しは、同じチャンネルのほかの送信が配っている途中のセッションを捨てない", async () => {
    const { worker } = fakeWorker();
    const first = fakeClient([null], [3]);
    const send = (client: ApiClient, messageId: string) =>
      sendEncryptedMessage(deps(worker, client), {
        channelId: CHANNEL,
        shapeEpoch: "3",
        messageId,
        parentId: null,
        body: "hello",
        mentions: NO_MENTIONS,
      });
    await send(first.client, "m0");

    // A の POST は応答を保留し、B の to_device も保留する。
    let rejectA!: (error: Error) => void;
    let releaseToDevice!: () => void;
    const toDeviceHeld = new Promise<void>((resolve) => {
      releaseToDevice = resolve;
    });
    let toDeviceCalls = 0;
    const posts: { epoch: number }[] = [];
    const client = {
      request: async () => ({
        channel_id: CHANNEL,
        membership_epoch: 4,
        user_ids: ["alice", "bob"],
        next_after: null,
      }),
      post: async (path: string, body: unknown) => {
        if (path === "/api/e2ee/keys/claim") return { data: { one_time_keys: {} }, txid: null };
        if (path === "/api/e2ee/to_device") {
          toDeviceCalls += 1;
          await toDeviceHeld;
          return { data: { undelivered: [] }, txid: null };
        }
        posts.push(body as { epoch: number });
        if (posts.length === 1) return new Promise((_, reject) => (rejectA = reject));
        return { data: {}, txid: "1" };
      },
    } as unknown as ApiClient;

    const a = send(client, "a");
    await vi.waitFor(() => expect(posts).toHaveLength(1));
    // B は、配り終えていない端末に鍵を配っている途中で止まる。
    (worker as unknown as { markUnshared: () => void }).markUnshared();
    const b = send(client, "b");
    await vi.waitFor(() => expect(toDeviceCalls).toBe(1));
    // そのあいだに A が 409 を受けても、B が用意したセッションは暗号化まで残る。
    rejectA(staleEpochError(4));
    await new Promise((resolve) => setTimeout(resolve, 0));
    releaseToDevice();
    await Promise.all([a, b]);
    expect(posts.map((post) => post.epoch)).toEqual([3, 3, 4]);
  });

  it("続けて送った 2 件がどちらも 409 を受けても、作り直したセッションは 1 つで、両方送れる", async () => {
    const { worker, ops } = fakeWorker();
    const { client, calls } = fakeClient(
      [staleEpochError(4), staleEpochError(4), null, null],
      [3, 4],
    );
    const send = (messageId: string) =>
      sendEncryptedMessage(deps(worker, client), {
        channelId: CHANNEL,
        shapeEpoch: "3",
        messageId,
        parentId: null,
        body: "hello",
        mentions: NO_MENTIONS,
      });
    await Promise.all([send("a"), send("b")]);
    const posts = calls.filter((call) => call.path === "/api/messages");
    expect(posts.map((call) => (call.body as { epoch: number }).epoch)).toEqual([3, 3, 4, 4]);
    // 後から 409 を受けた送信は、先の送信が作り直した epoch 4 のセッションを捨てずに使う。
    expect(ops.some((op) => op.op === "megolmDiscard")).toBe(false);
    expect(calls.filter((call) => call.path === "/api/e2ee/to_device")).toHaveLength(2);
  });

  it("初めて見た相手のピンを置けたら、同じスナップショットで判定し直してその端末に配る", async () => {
    const { worker, ops } = fakeWorker({ pendingPins: 1 });
    const { client, calls } = fakeClient([null], [3]);
    const summaries: RecipientSummary[] = [];
    await sendEncryptedMessage(
      { ...deps(worker, client), onRecipients: (_, summary) => summaries.push(summary) },
      {
        channelId: CHANNEL,
        shapeEpoch: "3",
        messageId: "m1",
        parentId: null,
        body: "hello",
        mentions: NO_MENTIONS,
      },
    );
    expect(calls.filter((call) => call.path.endsWith("/recipients"))).toHaveLength(1);
    expect(calls.filter((call) => call.path === "/api/e2ee/tofu_pins")).toHaveLength(1);
    const evaluations = ops.filter((op) => op.op === "megolmEvaluate");
    expect(evaluations).toHaveLength(2);
    // 配布先は、ピンを置いた相手の端末を入れた 2 回目の判定。
    expect(summaries).toHaveLength(1);
    expect(summaries[0]).toMatchObject({ newPins: [], deviceCount: 2 });
    const evaluated = ops.findIndex((op) => op === evaluations[1]);
    const started = ops.findIndex((op) => op.op === "megolmStart");
    expect(started).toBeGreaterThan(evaluated);
  });

  it("前回の判定から増えたメンバーを、配布先の表示に渡す", async () => {
    const { worker } = fakeWorker();
    const summaries: RecipientSummary[] = [];
    const send = async (epoch: number, messageId: string) => {
      const { client } = fakeClient([null], [epoch]);
      await sendEncryptedMessage(
        { ...deps(worker, client), onRecipients: (_, summary) => summaries.push(summary) },
        {
          channelId: CHANNEL,
          shapeEpoch: String(epoch),
          messageId,
          parentId: null,
          body: "hello",
          mentions: NO_MENTIONS,
        },
      );
    };
    await send(3, "m1");
    expect(summaries[0]!.addedUserIds).toEqual([]);
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

describe("editAlreadyApplied", () => {
  const stale = (current: number) =>
    new ApiError(
      409,
      "stale",
      { error: "stale", code: "E2EE_STALE_REVISION", current_revision: current },
      { path: "/api/messages/m1", method: "PATCH" },
    );
  const shown = (rowRevision: number, body: string) =>
    ({
      status: "decrypted",
      body,
      kind: "message",
      mentions: { user_ids: [], channel: false, here: false },
      sender: "verified",
      rowRevision,
    }) as const;

  it("今の版が自分の送った本文なら、応答をなくしただけとして済んだものにする", () => {
    expect(editAlreadyApplied(stale(2), shown(2, "edited"), "edited")).toBe(true);
  });

  it("本文や版が違う、版の警告がある、ほかのエラーなら、済んだものにしない", () => {
    expect(editAlreadyApplied(stale(2), shown(2, "other"), "edited")).toBe(false);
    expect(editAlreadyApplied(stale(2), shown(1, "edited"), "edited")).toBe(false);
    expect(
      editAlreadyApplied(
        stale(2),
        { ...shown(2, "edited"), revisionWarning: "conflict" },
        "edited",
      ),
    ).toBe(false);
    expect(editAlreadyApplied(staleEpochError(3), shown(2, "edited"), "edited")).toBe(false);
    expect(editAlreadyApplied(stale(2), undefined, "edited")).toBe(false);
  });
});
