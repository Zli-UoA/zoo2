import { createCollection, localOnlyCollectionOptions, queryOnce } from "@tanstack/react-db";
import { describe, expect, it } from "vite-plus/test";

import { messageFeedQuery } from "./messageFeed";
import type { MessageRow, UserRow } from "./types";

const users: UserRow[] = [
  {
    id: "user-1",
    user_name: "alice",
    display_name: "Alice",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
  {
    id: "user-2",
    user_name: "bob",
    display_name: "Bob",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
];

const messages: MessageRow[] = [
  {
    id: "message-1",
    user_id: "user-1",
    channel_id: "channel-1",
    content: "hello",
    created_at: "2026-01-02 00:00:00.000",
    updated_at: null,
    deleted_at: null,
  },
  {
    id: "message-2",
    user_id: "user-2",
    channel_id: "channel-1",
    content: "removed",
    created_at: "2026-01-03 00:00:00.000",
    updated_at: null,
    deleted_at: "2026-01-03 01:00:00.000",
  },
  {
    id: "message-3",
    user_id: "user-2",
    channel_id: "channel-1",
    content: "later",
    created_at: "2026-01-04 00:00:00.000",
    updated_at: null,
    deleted_at: null,
  },
];

let collectionSeq = 0;

function makeCollections(messageRows: MessageRow[], userRows: UserRow[]) {
  const seq = collectionSeq++;
  const messagesCollection = createCollection(
    localOnlyCollectionOptions<MessageRow>({
      id: `test-messages-${seq}`,
      getKey: (row) => row.id,
      initialData: messageRows,
    }),
  );
  const usersCollection = createCollection(
    localOnlyCollectionOptions<UserRow>({
      id: `test-users-${seq}`,
      getKey: (row) => row.id,
      initialData: userRows,
    }),
  );
  return { messages: messagesCollection, users: usersCollection };
}

describe("messageFeedQuery", () => {
  it("joins the author and excludes logically deleted messages", async () => {
    const result = await queryOnce((q) => messageFeedQuery(q, makeCollections(messages, users)));

    expect(result.map((row) => row.id)).toEqual(["message-1", "message-3"]);
    expect(result.find((row) => row.id === "message-1")?.display_name).toBe("Alice");
    expect(result.find((row) => row.id === "message-2")).toBeUndefined();
  });

  it("keeps messages whose author has not synced yet (left join)", async () => {
    const orphan: MessageRow = {
      id: "message-9",
      user_id: "user-missing",
      channel_id: "channel-1",
      content: "orphan",
      created_at: "2026-01-05 00:00:00.000",
      updated_at: null,
      deleted_at: null,
    };

    const result = await queryOnce((q) => messageFeedQuery(q, makeCollections([orphan], users)));

    expect(result).toHaveLength(1);
    expect(result[0].content).toBe("orphan");
    expect(result[0].user_name).toBeUndefined();
  });
});
