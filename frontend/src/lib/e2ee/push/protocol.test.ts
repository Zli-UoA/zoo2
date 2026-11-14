import { describe, expect, it, vi } from "vite-plus/test";

import {
  PUSH_DECRYPT_REQUEST,
  answerPushDecryptRequest,
  notificationBody,
  parsePushE2ee,
  type PushDecryptWorker,
} from "./protocol";

const BOB = "00000000-0000-4000-8000-00000000000b";
const MESSAGE = {
  channel_id: "00000000-0000-4000-8000-0000000000c1",
  message_id: "00000000-0000-4000-8000-0000000000d1",
  parent_id: null,
  user_id: "00000000-0000-4000-8000-00000000000a",
  message_kind: "message",
  epoch: 2,
  revision: 0,
  ciphertext: { algorithm: "m.megolm.v1.aes-sha2", sender_key: "k", session_id: "s" },
};

function port() {
  const replies: unknown[] = [];
  return {
    replies,
    port: { postMessage: (data: unknown) => replies.push(data) } as unknown as MessagePort,
  };
}

describe("parsePushE2ee", () => {
  it("サーバーの e2ee を zoo-crypto の行にする", () => {
    expect(parsePushE2ee({ recipient_id: BOB, message: MESSAGE })).toEqual({
      recipientId: BOB,
      message: MESSAGE,
    });
    // parent_id がなければ null にそろえる
    const { parent_id: _, ...withoutParent } = MESSAGE;
    expect(parsePushE2ee({ recipient_id: BOB, message: withoutParent })?.message.parent_id).toBe(
      null,
    );
  });

  it("形の違うものは復号しない", () => {
    for (const value of [
      undefined,
      "x",
      { recipient_id: "bob", message: MESSAGE },
      { recipient_id: BOB },
      { recipient_id: BOB, message: { ...MESSAGE, epoch: "2" } },
      { recipient_id: BOB, message: { ...MESSAGE, revision: 1.5 } },
      { recipient_id: BOB, message: { ...MESSAGE, parent_id: 1 } },
      { recipient_id: BOB, message: { ...MESSAGE, ciphertext: "c" } },
      { recipient_id: BOB, message: { ...MESSAGE, ciphertext: { session_id: "s" } } },
    ]) {
      expect(parsePushE2ee(value)).toBeNull();
    }
  });
});

describe("notificationBody", () => {
  it("コードポイントで 240 文字に切る（サーバーの chars().take(240) と同じ）", () => {
    expect(notificationBody("🦊".repeat(300))).toBe("🦊".repeat(240));
    expect(notificationBody("short")).toBe("short");
  });
});

describe("answerPushDecryptRequest", () => {
  const request = { type: PUSH_DECRYPT_REQUEST, namespace: `user-${BOB}`, message: MESSAGE };

  it("暗号ストアを開いているタブは、Worker で復号して本文を返す", async () => {
    const { port: reply, replies } = port();
    const decryptRows = vi.fn(async () => [
      { status: "decrypted" as const, body: "あ".repeat(300) },
    ]);
    const worker: PushDecryptWorker = { request: decryptRows };
    expect(answerPushDecryptRequest(request, reply, { worker, namespace: `user-${BOB}` })).toBe(
      true,
    );
    await vi.waitFor(() => expect(replies).toEqual([{ body: "あ".repeat(240) }]));
    expect(decryptRows).toHaveBeenCalledWith({ op: "megolmDecrypt", rows: [MESSAGE] });
  });

  it("開いていない、別のユーザー、復号できないときは null を返して待たせない", async () => {
    const undecryptable: PushDecryptWorker = {
      request: async () => [{ status: "undecryptable" as const }],
    };
    const failing: PushDecryptWorker = {
      request: async () => {
        throw new Error("closed");
      },
    };
    const cases = [
      { worker: null, namespace: `user-${BOB}` },
      { worker: undecryptable, namespace: "user-other" },
      { worker: undecryptable, namespace: `user-${BOB}` },
      { worker: failing, namespace: `user-${BOB}` },
    ];
    for (const current of cases) {
      const { port: reply, replies } = port();
      answerPushDecryptRequest(request, reply, current);
      await vi.waitFor(() => expect(replies).toEqual([{ body: null }]));
    }
  });

  it("依頼でないメッセージには答えない", () => {
    const { port: reply, replies } = port();
    const current = { worker: null, namespace: `user-${BOB}` };
    expect(answerPushDecryptRequest({ type: "AVATAR_UPDATED" }, reply, current)).toBe(false);
    expect(answerPushDecryptRequest(request, undefined, current)).toBe(false);
    expect(replies).toEqual([]);
  });
});
