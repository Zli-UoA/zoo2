import { describe, expect, it, vi } from "vite-plus/test";

import { createApiClient, type ApiClient } from "../api/client";
import type { ChannelRow, ChannelUserRow, MessageRow } from "./types";
import {
  createChannelUserWrites,
  createChannelWrites,
  createMessageWrites,
  createReactionWrites,
  createNotificationWrites,
} from "./writes";

const TXID = "4242";
const CURRENT_USER = "u1";

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
  parent_id: null,
};

const channel: ChannelRow = {
  id: "c1",
  channel_name: "general",
  description: null,
  created_user_id: "u1",
  is_private: false,
  is_dm: false,
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
    const writes = createMessageWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
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

  it("sends parent_id for thread replies", async () => {
    const { client, post } = makeClient();
    const writes = createMessageWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection } = makeCollection();
    const reply = { ...message, id: "m2", parent_id: "m1" };

    await writes.onInsert(params(collection, [insertMutation(reply)]));

    expect(post).toHaveBeenCalledWith("/api/messages", {
      id: "m2",
      channel_id: "c1",
      content: "hello",
      parent_id: "m1",
    });
  });

  it("PATCHes content edits", async () => {
    const { client, patch } = makeClient();
    const writes = createMessageWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection } = makeCollection();

    await writes.onUpdate(
      params(collection, [updateMutation(message, { content: "edited", updated_at: "now" })]),
    );

    expect(patch).toHaveBeenCalledWith("/api/messages/m1", { content: "edited" });
  });

  it("routes a deleted_at update to DELETE as a logical delete", async () => {
    const { client, del, patch } = makeClient();
    const writes = createMessageWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
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
    const writes = createMessageWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection, awaitTxId } = makeCollection();

    await writes.onDelete(params(collection, [deleteMutation(message)]));

    expect(del).toHaveBeenCalledWith("/api/messages/m1");
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });

  it("propagates REST failures so TanStack DB rolls back the optimistic state", async () => {
    const { client, post } = makeClient();
    post.mockRejectedValueOnce(new Error("403 forbidden"));
    const writes = createMessageWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection, awaitTxId } = makeCollection();

    await expect(writes.onInsert(params(collection, [insertMutation(message)]))).rejects.toThrow(
      "403 forbidden",
    );
    expect(awaitTxId).not.toHaveBeenCalled();
  });

  it("skips the handshake in skip mode", async () => {
    const { client } = makeClient();
    const writes = createMessageWrites({
      client,
      txidMode: "skip",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection, awaitTxId, awaitMatch } = makeCollection();

    await writes.onInsert(params(collection, [insertMutation(message)]));

    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).not.toHaveBeenCalled();
  });

  it("waits via awaitMatch with an insert matcher in awaitMatch mode", async () => {
    const { client } = makeClient();
    const writes = createMessageWrites({
      client,
      txidMode: "awaitMatch",
      getCurrentUserId: () => CURRENT_USER,
    });
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
    const writes = createChannelWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
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
    const writes = createChannelWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection } = makeCollection();

    await writes.onUpdate(
      params(collection, [updateMutation(channel, { channel_name: "random" })]),
    );

    expect(patch).toHaveBeenCalledWith("/api/channels/c1", { channel_name: "random" });
  });

  it("PATCHes the description and can clear it with an empty string", async () => {
    const { client, patch } = makeClient();
    const writes = createChannelWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection } = makeCollection();

    await writes.onUpdate(params(collection, [updateMutation(channel, { description: "" })]));

    expect(patch).toHaveBeenCalledWith("/api/channels/c1", { description: "" });
  });
});

describe("channel_user writes", () => {
  it("POSTs /join by default", async () => {
    const { client, post } = makeClient();
    const writes = createChannelUserWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection, awaitTxId } = makeCollection();

    await writes.onInsert(params(collection, [insertMutation(membership)]));

    expect(post).toHaveBeenCalledWith("/api/channels/c1/join");
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });

  it("POSTs /users when the insert metadata asks to add a member", async () => {
    const { client, post } = makeClient();
    const writes = createChannelUserWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection } = makeCollection();

    await writes.onInsert(
      params(collection, [
        insertMutation(membership, { action: "add", user_id: membership.user_id }),
      ]),
    );

    expect(post).toHaveBeenCalledWith("/api/channels/c1/users", { user_id: "u1" });
  });

  it("POSTs /leave on delete of the current user's own row", async () => {
    const { client, post } = makeClient();
    const writes = createChannelUserWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection, awaitTxId } = makeCollection();

    await writes.onDelete(params(collection, [deleteMutation(membership)]));

    expect(post).toHaveBeenCalledWith("/api/channels/c1/leave");
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });

  it("refuses to leave another member's row so the optimistic state rolls back", async () => {
    const { client, post } = makeClient();
    const writes = createChannelUserWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    const { collection, awaitTxId } = makeCollection();

    await expect(
      writes.onDelete(params(collection, [deleteMutation({ ...membership, user_id: "u2" })])),
    ).rejects.toThrow(/current user/);
    expect(post).not.toHaveBeenCalled();
    expect(awaitTxId).not.toHaveBeenCalled();
  });

  it("reads the current user at delete time so OIDC can resolve it after collection setup", async () => {
    const { client, post } = makeClient();
    let currentUserId: string | null = null;
    const writes = createChannelUserWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => currentUserId,
    });
    const { collection } = makeCollection();

    // 解決前（null）は本人確認できないため拒否する。
    await expect(writes.onDelete(params(collection, [deleteMutation(membership)]))).rejects.toThrow(
      /current user/,
    );
    expect(post).not.toHaveBeenCalled();

    // /api/users/me の解決後に押された退出は通る。
    currentUserId = CURRENT_USER;
    await writes.onDelete(params(collection, [deleteMutation(membership)]));
    expect(post).toHaveBeenCalledWith("/api/channels/c1/leave");
  });
});

describe("reaction writes", () => {
  const reaction = {
    id: "r1",
    message_id: "m1",
    user_id: CURRENT_USER,
    emoji: "kitchen:1f431_1f600",
    created_at: "now",
  };
  it("sends a stable id and waits for its INSERT in awaitMatch mode", async () => {
    const { client, post } = makeClient();
    post.mockResolvedValueOnce({ data: reaction, txid: TXID });
    const { collection, awaitMatch } = makeCollection();
    const writes = createReactionWrites({
      client,
      txidMode: "awaitMatch",
      getCurrentUserId: () => CURRENT_USER,
    });
    await writes.onInsert(params(collection, [insertMutation(reaction)]));
    expect(post).toHaveBeenCalledWith("/api/messages/m1/reactions", {
      id: "r1",
      emoji: reaction.emoji,
    });
    const matcher = awaitMatch.mock.calls[0][0];
    expect(
      matcher({
        key: "reaction-key",
        headers: { operation: "insert" },
        value: reaction,
      }),
    ).toBe(true);
    expect(
      matcher({
        key: "reaction-key",
        headers: { operation: "insert" },
        value: { ...reaction, id: "someone-else" },
      }),
    ).toBe(false);
  });
  it("does not wait for a nonexistent INSERT when another tab added the reaction", async () => {
    const { client, post } = makeClient();
    post.mockResolvedValueOnce({ data: { ...reaction, id: "existing-id" }, txid: TXID });
    const { collection, awaitTxId, awaitMatch } = makeCollection();
    const writes = createReactionWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    await writes.onInsert(params(collection, [insertMutation(reaction)]));
    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).not.toHaveBeenCalled();
  });
  it("accepts the backend's null txid through the real API client for a duplicate reaction", async () => {
    const client = createApiClient({
      getToken: () => null,
      fetch: async () =>
        new Response(JSON.stringify({ data: { ...reaction, id: "existing-id" }, txid: null }), {
          headers: { "Content-Type": "application/json" },
        }),
    });
    const { collection, awaitTxId, awaitMatch } = makeCollection();
    const writes = createReactionWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    await writes.onInsert(params(collection, [insertMutation(reaction)]));
    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).not.toHaveBeenCalled();
  });

  it("encodes the emoji when removing my reaction and waits for the txid", async () => {
    const { client, del } = makeClient();
    const { collection, awaitTxId } = makeCollection();
    const writes = createReactionWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    await writes.onDelete(params(collection, [deleteMutation(reaction)]));
    expect(del).toHaveBeenCalledWith("/api/messages/m1/reactions?emoji=kitchen%3A1f431_1f600");
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });
  it("rejects another person's removal and propagates HTTP errors for rollback", async () => {
    const { client, del, post } = makeClient();
    const { collection } = makeCollection();
    const writes = createReactionWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    await expect(
      writes.onDelete(
        params(collection, [deleteMutation({ ...reaction, user_id: "someone-else" })]),
      ),
    ).rejects.toThrow("自分のリアクション");
    expect(del).not.toHaveBeenCalled();
    post.mockRejectedValueOnce(new Error("403 forbidden"));
    await expect(writes.onInsert(params(collection, [insertMutation(reaction)]))).rejects.toThrow(
      "403 forbidden",
    );
  });
});

describe("notification writes", () => {
  const notification = { id: "n1", user_id: CURRENT_USER, is_read: false };
  it("marks an individual notification read and waits for replication", async () => {
    const { client, patch, post } = makeClient();
    const { collection, awaitTxId } = makeCollection();
    const writes = createNotificationWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    await writes.onUpdate(params(collection, [updateMutation(notification, { is_read: true })]));
    expect(patch).toHaveBeenCalledWith("/api/notifications/n1/read");
    expect(post).not.toHaveBeenCalled();
    expect(awaitTxId).toHaveBeenCalledWith(Number(TXID), undefined);
  });
  it("uses the bulk API and propagates failures for optimistic rollback", async () => {
    const { client, post, patch } = makeClient();
    const { collection } = makeCollection();
    const writes = createNotificationWrites({
      client,
      txidMode: "skip",
      getCurrentUserId: () => CURRENT_USER,
    });
    const mutations = [
      updateMutation(notification, { is_read: true }),
      updateMutation({ ...notification, id: "n2" }, { is_read: true }),
    ];
    await writes.onUpdate(params(collection, mutations));
    expect(post).toHaveBeenCalledWith("/api/notifications/read-all");
    expect(patch).not.toHaveBeenCalled();
    post.mockRejectedValueOnce(new Error("offline"));
    await expect(writes.onUpdate(params(collection, mutations))).rejects.toThrow("offline");
  });
  it("does not write another user's notification", async () => {
    const { client, patch, post } = makeClient();
    const { collection } = makeCollection();
    const writes = createNotificationWrites({
      client,
      txidMode: "skip",
      getCurrentUserId: () => "other",
    });
    await expect(
      writes.onUpdate(params(collection, [updateMutation(notification, { is_read: true })])),
    ).rejects.toThrow(/own notifications/);
    expect(patch).not.toHaveBeenCalled();
    expect(post).not.toHaveBeenCalled();
  });
  it("does not wait for replication on an already-read no-op", async () => {
    const { client, patch } = makeClient();
    patch.mockResolvedValueOnce({ data: {}, txid: null } as never);
    const { collection, awaitTxId, awaitMatch } = makeCollection();
    const writes = createNotificationWrites({
      client,
      txidMode: "awaitTxId",
      getCurrentUserId: () => CURRENT_USER,
    });
    await writes.onUpdate(params(collection, [updateMutation(notification, { is_read: true })]));
    expect(awaitTxId).not.toHaveBeenCalled();
    expect(awaitMatch).not.toHaveBeenCalled();
  });
});
