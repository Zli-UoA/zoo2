import { createCollection, localOnlyCollectionOptions, queryOnce } from "@tanstack/react-db";
import { describe, expect, it } from "vite-plus/test";

import { messageFeedQuery, replyParentQuery } from "./messageFeed";
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
    parent_id: null,
  },
  {
    id: "message-2",
    user_id: "user-2",
    channel_id: "channel-1",
    content: "removed",
    created_at: "2026-01-03 00:00:00.000",
    updated_at: null,
    deleted_at: "2026-01-03 01:00:00.000",
    parent_id: null,
  },
  {
    id: "message-3",
    user_id: "user-2",
    channel_id: "channel-1",
    content: "later",
    created_at: "2026-01-04 00:00:00.000",
    updated_at: null,
    deleted_at: null,
    parent_id: null,
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

  it("drops a message from the feed after a logical delete update", async () => {
    const { messages: messagesCollection, users: usersCollection } = makeCollections(
      messages,
      users,
    );

    const before = await queryOnce((q) =>
      messageFeedQuery(q, { messages: messagesCollection, users: usersCollection }),
    );
    expect(before.map((row) => row.id)).toContain("message-1");

    messagesCollection.update("message-1", (draft) => {
      draft.deleted_at = "2026-01-02 12:00:00.000";
    });

    const after = await queryOnce((q) =>
      messageFeedQuery(q, { messages: messagesCollection, users: usersCollection }),
    );
    expect(after.map((row) => row.id)).not.toContain("message-1");
  });

  it("keeps messages whose author has not synced yet (left join)", async () => {
    const orphan: MessageRow = {
      id: "message-9",
      user_id: "user-missing",
      channel_id: "channel-1",
      content: "orphan",
      created_at: "2026-01-05 00:00:00.000",
      updated_at: null,
      parent_id: null,
      deleted_at: null,
    };

    const result = await queryOnce((q) => messageFeedQuery(q, makeCollections([orphan], users)));

    expect(result).toHaveLength(1);
    expect(result[0].content).toBe("orphan");
    expect(result[0].user_name).toBeUndefined();
  });
});

describe("messageFeedQuery scopes", () => {
  const withReplies: MessageRow[] = [
    {
      id: "root-1",
      user_id: "user-1",
      channel_id: "channel-1",
      content: "root",
      created_at: "2026-01-02 00:00:00.000",
      updated_at: null,
      deleted_at: null,
      parent_id: null,
    },
    {
      id: "reply-1",
      user_id: "user-2",
      channel_id: "channel-1",
      content: "reply",
      created_at: "2026-01-02 00:01:00.000",
      updated_at: null,
      deleted_at: null,
      parent_id: "root-1",
    },
    {
      id: "reply-deleted",
      user_id: "user-2",
      channel_id: "channel-1",
      content: "removed reply",
      created_at: "2026-01-02 00:02:00.000",
      updated_at: null,
      deleted_at: "2026-01-02 00:03:00.000",
      parent_id: "root-1",
    },
  ];

  it("returns only top-level messages in the default scope", async () => {
    const result = await queryOnce((q) => messageFeedQuery(q, makeCollections(withReplies, users)));
    expect(result.map((row) => row.id)).toEqual(["root-1"]);
  });

  it("returns only the replies of the requested root", async () => {
    const result = await queryOnce((q) =>
      messageFeedQuery(q, makeCollections(withReplies, users), { parentId: "root-1" }),
    );
    expect(result.map((row) => row.id)).toEqual(["reply-1"]);
    expect(result[0]?.parent_id).toBe("root-1");
  });

  it("counts non-deleted reply parents", async () => {
    const result = await queryOnce((q) =>
      replyParentQuery(q, { messages: makeCollections(withReplies, users).messages }),
    );
    expect(result.map((row) => row.parent_id)).toEqual(["root-1"]);
  });
});

describe("messageFeedQuery since window", () => {
  it("returns only messages at or after the window start", async () => {
    const result = await queryOnce((q) =>
      messageFeedQuery(q, makeCollections(messages, users), {
        parentId: null,
        since: "2026-01-03 00:00:00.000",
      }),
    );
    expect(result.map((row) => row.id)).toEqual(["message-3"]);
  });

  it("applies the same window to thread replies", async () => {
    const replies: MessageRow[] = [
      {
        id: "reply-old",
        user_id: "user-1",
        channel_id: "channel-1",
        content: "old",
        created_at: "2026-01-01 00:00:00.000",
        updated_at: null,
        deleted_at: null,
        parent_id: "root-1",
      },
      {
        id: "reply-new",
        user_id: "user-1",
        channel_id: "channel-1",
        content: "new",
        created_at: "2026-01-05 00:00:00.000",
        updated_at: null,
        deleted_at: null,
        parent_id: "root-1",
      },
    ];
    const result = await queryOnce((q) =>
      messageFeedQuery(q, makeCollections(replies, users), {
        parentId: "root-1",
        since: "2026-01-02 00:00:00.000",
      }),
    );
    expect(result.map((row) => row.id)).toEqual(["reply-new"]);
  });
});
