import { describe, expect, it, vi } from "vite-plus/test";

import type { ApiClient } from "../api/client";
import type { ChannelRow, ChannelUserRow, MessageRow } from "./types";
import { createChannelUserWrites, createChannelWrites, createMessageWrites } from "./writes";

const TXID = "18446744073709551615";

function makeClient() {
  const post = vi.fn(async () => ({ data: {}, txid: TXID }));
  const patch = vi.fn(async () => ({ data: {}, txid: TXID }));
  const del = vi.fn(async () => ({ data: {}, txid: TXID }));
  return { client: { post, patch, delete: del } as unknown as ApiClient, post, patch, del };
}

function makeCollection() {
  const awaitTxId = vi.fn<(txid: number, timeout?: number) => Promise<boolean>>(async () => true);
  const awaitMatch = vi.fn<
    (matchFn: (message: unknown) => boolean, timeout?: number) => Promise<boolean>
  >(async () => true);
  return { collection: { utils: { awaitTxId, awaitMatch } } as never, awaitTxId, awaitMatch };
}

function params(collection: unknown, mutations: unknown[]) {
  return { transaction: { mutations }, collection } as never;
}

function insertMutation<T>(modified: T, metadata?: unknown) {
  return { modified, changes: modified, original: {}, metadata };
}

function updateMutation<T>(original: T, changes: Partial<T>) {
  return { modified: { ...original, ...changes }, changes, original };
}

function deleteMutation<T>(original: T) {
  return { modified: original, changes: original, original };
}

const message: MessageRow = {
  id: "m1",
  user_id: "u1",
  channel_id: "c1",
  content: "hello",
  created_at: "2026-01-01 00:00:00.000",
  updated_at: null,
  deleted_at: null,
};

const channel: ChannelRow = {
  id: "c1",
  channel_name: "general",
  description: null,
  created_user_id: "u1",
  is_private: false,
  created_at: "2026-01-01 00:00:00.000",
  updated_at: null,
  archive_at: null,
  deleted_at: null,
};

const membership: ChannelUserRow = {
  user_id: "u1",
  channel_id: "c1",
  joined_at: "2026-01-01 00:00:00.000",
};

describe("message writes", () => {
  it("POSTs the message and awaits the numeric txid", async () => {
    const { client, post } = makeClient();
    const writes = createMessageWrites({ client, txidMode: "awaitTxId" });
    const { collection, awaitTxId, awaitMatch } = makeCollection();

    await writes.onInsert(params(collection, [insertMutation(message)]));

    expect(post).toHaveBeenCalledWith("/api/messages", {
      id: "m1",
      channel_id: "c1",
      content: "hello",
    });
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
    expect(awaitMatch).not.toHaveBeenCalled();
  });

  it("PATCHes content edits", async () => {
    const { client, patch } = makeClient();
    const writes = createMessageWrites({ client, txidMode: "awaitTxId" });
    const { collection } = makeCollection();

    await writes.onUpdate(
      params(collection, [updateMutation(message, { content: "edited", updated_at: "now" })]),
    );

    expect(patch).toHaveBeenCalledWith("/api/messages/m1", { content: "edited" });
  });

  it("routes a deleted_at update to DELETE as a logical delete", async () => {
    const { client, del, patch } = makeClient();
    const writes = createMessageWrites({ client, txidMode: "awaitTxId" });
    const { collection, awaitTxId } = makeCollection();

    await writes.onUpdate(
      params(collection, [updateMutation(message, { deleted_at: "2026-01-02 00:00:00.000" })]),
    );

    expect(del).toHaveBeenCalledWith("/api/messages/m1");
    expect(patch).not.toHaveBeenCalled();
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });

  it("treats onDelete as a logical delete via DELETE", async () => {
    const { client, del } = makeClient();
    const writes = createMessageWrites({ client, txidMode: "awaitTxId" });
    const { collection, awaitTxId } = makeCollection();

    await writes.onDelete(params(collection, [deleteMutation(message)]));

    expect(del).toHaveBeenCalledWith("/api/messages/m1");
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });

  it("propagates REST failures so TanStack DB rolls back the optimistic state", async () => {
    const { client, post } = makeClient();
    post.mockRejectedValueOnce(new Error("403 forbidden"));
    const writes = createMessageWrites({ client, txidMode: "awaitTxId" });
    const { collection, awaitTxId } = makeCollection();

    await expect(writes.onInsert(params(collection, [insertMutation(message)]))).rejects.toThrow(
      "403 forbidden",
    );
    expect(awaitTxId).not.toHaveBeenCalled();
  });

  it("skips the handshake in skip mode", async () => {
    const { client } = makeClient();
    const writes = createMessageWrites({ client, txidMode: "skip" });
    const { collection, awaitTxId, awaitMatch } = makeCollection();

    await writes.onInsert(params(collection, [insertMutation(message)]));

    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).not.toHaveBeenCalled();
  });

  it("waits via awaitMatch with an insert matcher in awaitMatch mode", async () => {
    const { client } = makeClient();
    const writes = createMessageWrites({ client, txidMode: "awaitMatch" });
    const { collection, awaitTxId, awaitMatch } = makeCollection();

    await writes.onInsert(params(collection, [insertMutation(message)]));

    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).toHaveBeenCalledTimes(1);
    const [match] = awaitMatch.mock.calls[0]!;
    const matcher = match as (message: unknown) => boolean;
    expect(matcher({ key: "m1", value: { id: "m1" }, headers: { operation: "insert" } })).toBe(
      true,
    );
    expect(matcher({ key: "m1", value: { id: "m1" }, headers: { operation: "update" } })).toBe(
      false,
    );
  });
});

describe("channel writes", () => {
  it("POSTs the channel and awaits the txid", async () => {
    const { client, post } = makeClient();
    const writes = createChannelWrites({ client, txidMode: "awaitTxId" });
    const { collection, awaitTxId } = makeCollection();

    await writes.onInsert(params(collection, [insertMutation(channel)]));

    expect(post).toHaveBeenCalledWith("/api/channels", {
      id: "c1",
      channel_name: "general",
      description: null,
      is_private: false,
    });
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });

  it("PATCHes only the changed channel fields", async () => {
    const { client, patch } = makeClient();
    const writes = createChannelWrites({ client, txidMode: "awaitTxId" });
    const { collection } = makeCollection();

    await writes.onUpdate(
      params(collection, [updateMutation(channel, { channel_name: "random" })]),
    );

    expect(patch).toHaveBeenCalledWith("/api/channels/c1", { channel_name: "random" });
  });
});

describe("channel_user writes", () => {
  it("POSTs /join by default", async () => {
    const { client, post } = makeClient();
    const writes = createChannelUserWrites({ client, txidMode: "awaitTxId" });
    const { collection, awaitTxId } = makeCollection();

    await writes.onInsert(params(collection, [insertMutation(membership)]));

    expect(post).toHaveBeenCalledWith("/api/channels/c1/join");
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });

  it("POSTs /users when the insert metadata asks to add a member", async () => {
    const { client, post } = makeClient();
    const writes = createChannelUserWrites({ client, txidMode: "awaitTxId" });
    const { collection } = makeCollection();

    await writes.onInsert(
      params(collection, [
        insertMutation(membership, { action: "add", user_id: membership.user_id }),
      ]),
    );

    expect(post).toHaveBeenCalledWith("/api/channels/c1/users", { user_id: "u1" });
  });

  it("POSTs /leave on delete", async () => {
    const { client, post } = makeClient();
    const writes = createChannelUserWrites({ client, txidMode: "awaitTxId" });
    const { collection, awaitTxId } = makeCollection();

    await writes.onDelete(params(collection, [deleteMutation(membership)]));

    expect(post).toHaveBeenCalledWith("/api/channels/c1/leave");
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });
});
