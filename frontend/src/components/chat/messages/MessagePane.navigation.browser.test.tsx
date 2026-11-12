import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { MessagePane } from "./MessagePane";

const fixture = vi.hoisted(() => {
  const root = {
    id: "old-root",
    channel_id: "general",
    user_id: "alice",
    content: "Old thread",
    parent_id: null as string | null,
    created_at: "2020-01-01 00:00:00.000",
    updated_at: null,
    deleted_at: null as string | null,
    thread_archive_at: null as string | null,
    user_name: "alice",
    display_name: "Alice",
  };
  return {
    root,
    /** `useThreadRoot` が返す行 (取得が終わるまでは、空)。 */
    rootRows: [root] as (typeof root)[],
    rootLoading: false,
    /** 通知・リンクの対象 (`?message=`) として返す行。 */
    targetRows: [] as (typeof root)[],
    /** メイン一覧の行と、古いページの有無。 */
    feed: [] as (typeof root)[],
    feedLoading: false,
    hasOlder: false,
    isLoadingOlder: false,
    loadOlder: vi.fn(),
    replies: [] as (typeof root)[],
    insert: vi.fn(),
    empty: [],
    users: [] as { id: string; display_name: string; user_name: string }[],
    reactions: [] as {
      id: string;
      message_id: string;
      user_id: string;
      emoji: string;
      created_at: string;
    }[],
    closeThread: vi.fn(),
    openThread: vi.fn(),
    api: { request: vi.fn(), post: vi.fn().mockResolvedValue({}) },
  };
});
const collection = {
  insert: fixture.insert,
  get: () => undefined,
  values: () => fixture.empty.values(),
};
vi.mock("../../../lib/electric/collections-context", () => ({
  useCollections: () => ({
    messages: () => collection,
    reactions: () => collection,
    attachments: () => collection,
    channelUsers: {},
    config: {},
  }),
}));
vi.mock("../../../lib/api/client", async (original) => ({
  ...(await original<typeof import("../../../lib/api/client")>()),
  createConfiguredApiClient: () => fixture.api,
}));
vi.mock("../../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "alice" }),
}));
vi.mock("../../../hooks/useKitchenCatalog", () => ({
  useKitchenCatalog: () => ({ catalog: null }),
}));
vi.mock("../../../hooks/useRealtime", () => ({
  useUsers: () => ({ data: fixture.users, isLoading: false }),
  useChannelMembers: () => ({ data: fixture.empty }),
  useMessageReactions: () => ({ data: fixture.reactions, isLoading: false, isError: false }),
  useMessageFeed: () => ({
    data: fixture.feed,
    isLoading: fixture.feedLoading,
    isInitialSyncLoading: false,
    isError: false,
    hasOlder: fixture.hasOlder,
    isLoadingOlder: fixture.isLoadingOlder,
    loadOlder: fixture.loadOlder,
  }),
}));
vi.mock("../../../hooks/useAttachments", () => ({
  useMessageAttachments: () => ({ data: fixture.empty }),
  useAttachmentUpload: () => vi.fn(),
}));
vi.mock("../../../hooks/useThread", () => ({
  useMessageById: () => ({ data: fixture.targetRows, isLoading: false }),
  useThreadRoot: () => ({ data: fixture.rootRows, isLoading: fixture.rootLoading }),
  useThreadFeed: () => ({
    data: fixture.replies,
    isLoading: false,
    isError: false,
    hasOlder: false,
    isLoadingOlder: false,
    loadOlder: vi.fn(),
  }),
}));
vi.mock("../../../hooks/useTyping", () => ({
  useTyping: () => ({ users: fixture.empty, onTyping: () => {} }),
}));
vi.mock("../../../hooks/useWriteRunner", () => ({
  useWriteRunner: () => ({ error: null, setError: () => {}, run: vi.fn(), isPending: false }),
}));
vi.mock("../../../lib/offline/offline-context", () => ({
  useOfflineStatus: () => ({ isOnline: true }),
}));
vi.mock("../../layout/shell-context", () => ({
  useShell: () => ({
    threadRootId: fixture.root.id,
    openThread: fixture.openThread,
    closeThread: fixture.closeThread,
  }),
}));
vi.mock("./MessageList", () => ({
  MessageList: ({
    reactions,
  }: {
    reactions: Map<string, { userNames?: Record<string, string> }[]>;
  }) => <div>{reactions.get("old-root")?.[0]?.userNames?.alice}</div>,
}));
vi.mock("./ThreadPane", () => ({
  ThreadPane: ({
    root,
    replies,
    readOnly,
    isMember,
    onSendReply,
  }: {
    root: { content: string };
    replies: { id: string; content: string }[];
    readOnly?: boolean;
    isMember?: boolean;
    onSendReply: (content: string) => Promise<boolean>;
  }) => (
    <div>
      {root.content}
      {replies.map((reply) => (
        <p key={reply.id}>{reply.content}</p>
      ))}
      {!readOnly ? (
        <button disabled={!isMember} onClick={() => void onSendReply("Continued reply")}>
          Reply to thread
        </button>
      ) : null}
    </div>
  ),
}));
vi.mock("./ReactionPicker", () => ({ ReactionPicker: () => null }));

const pane = (targetMessageId?: string) => (
  <LocaleProvider>
    <MessagePane channelId="general" isMember={false} targetMessageId={targetMessageId} />
  </LocaleProvider>
);

beforeEach(() => {
  fixture.root.thread_archive_at = null;
  fixture.replies = [];
  fixture.insert.mockClear();
  fixture.api.post.mockClear();
  fixture.root.deleted_at = null;
  fixture.root.parent_id = null;
  fixture.rootRows = [fixture.root];
  fixture.rootLoading = false;
  fixture.targetRows = [];
  fixture.feed = [];
  fixture.feedLoading = false;
  fixture.hasOlder = false;
  fixture.isLoadingOlder = false;
  fixture.users = [];
  fixture.reactions = [];
  fixture.closeThread.mockClear();
  fixture.openThread.mockClear();
  fixture.loadOlder.mockClear();
});

describe("links and threads whose rows are older than the loaded range", () => {
  it("keeps older replies visible after an archived thread is reopened", async () => {
    fixture.root.thread_archive_at = new Date().toISOString();
    fixture.replies = [40, 10].map((days) => ({
      ...fixture.root,
      id: `reply-${days}`,
      content: `Reply ${days} days ago`,
      parent_id: fixture.root.id,
      created_at: new Date(Date.now() - days * 86400000).toISOString(),
      thread_archive_at: null,
    }));
    const view = () => (
      <LocaleProvider>
        <MessagePane channelId="general" isMember />
      </LocaleProvider>
    );
    const { rerender } = render(view());
    await screen.findByText("Reply 40 days ago");
    screen.getByRole("button", { name: "Reply to thread" }).click();
    await waitFor(() =>
      expect(fixture.insert).toHaveBeenCalledWith(
        expect.objectContaining({ parent_id: fixture.root.id, content: "Continued reply" }),
        undefined,
      ),
    );
    expect(fixture.api.post).not.toHaveBeenCalled();
    // 返信の保存後に Electric から再開状態が届く。
    fixture.root.thread_archive_at = null;
    rerender(view());
    await waitFor(() => {
      expect(screen.getByText("Reply 40 days ago")).toBeTruthy();
      expect(screen.getByText("Reply 10 days ago")).toBeTruthy();
    });
  });
  it("opens the thread of a linked reply once the reply row is loaded", async () => {
    const reply = { ...fixture.root, id: "old-reply", parent_id: "old-root" };
    const { rerender } = render(pane("old-reply"));
    expect(fixture.openThread).not.toHaveBeenCalled();
    fixture.targetRows = [reply];
    rerender(pane("old-reply"));
    await waitFor(() => expect(fixture.openThread).toHaveBeenCalledWith("old-root"));
    expect(fixture.closeThread).not.toHaveBeenCalled();
  });

  it("does not close a thread whose root is still being fetched", async () => {
    fixture.rootRows = [];
    fixture.rootLoading = true;
    const { rerender } = render(pane());
    expect(fixture.closeThread).not.toHaveBeenCalled();
    // 取得が終わっても、その行がなければ、(存在しないので) 閉じる
    fixture.rootLoading = false;
    rerender(pane());
    await waitFor(() => expect(fixture.closeThread).toHaveBeenCalled());
  });

  it("keeps an old root visible even though it is not in the main feed", async () => {
    render(pane());
    await screen.findByText("Old thread");
    expect(fixture.closeThread).not.toHaveBeenCalled();
  });

  it("keeps a reply visible when it becomes the root of a child thread", async () => {
    fixture.root.parent_id = "ancestor";
    render(pane());
    await screen.findByText("Old thread");
    expect(fixture.closeThread).not.toHaveBeenCalled();
  });

  it("closes the thread when its root was deleted", async () => {
    fixture.root.deleted_at = "2020-01-02 00:00:00.000";
    render(pane());
    await waitFor(() => expect(fixture.closeThread).toHaveBeenCalled());
  });

  it("loads older pages until a linked top-level message is in the feed", async () => {
    const target = { ...fixture.root, id: "old-top", created_at: "2019-01-01 00:00:00.000" };
    fixture.targetRows = [target];
    fixture.hasOlder = true;
    const { rerender } = render(pane("old-top"));
    await waitFor(() => expect(fixture.loadOlder).toHaveBeenCalledTimes(1));
    // 取得中は重ねて要求しない
    fixture.isLoadingOlder = true;
    rerender(pane("old-top"));
    expect(fixture.loadOlder).toHaveBeenCalledTimes(1);
    // 1 ページ目に入っていなければ、次のページを要求する
    fixture.isLoadingOlder = false;
    fixture.feed = [{ ...fixture.root, id: "newer" }];
    rerender(pane("old-top"));
    await waitFor(() => expect(fixture.loadOlder).toHaveBeenCalledTimes(2));
    // 対象が入ったら、止まる
    fixture.feed = [target];
    rerender(pane("old-top"));
    expect(fixture.loadOlder).toHaveBeenCalledTimes(2);
  });

  it("stops loading when there are no older messages", async () => {
    fixture.targetRows = [{ ...fixture.root, id: "old-top" }];
    fixture.hasOlder = false;
    render(pane("old-top"));
    expect(fixture.loadOlder).not.toHaveBeenCalled();
  });

  it("passes resolved reaction user names to the message list", async () => {
    fixture.users = [{ id: "alice", display_name: "Alice", user_name: "alice" }];
    fixture.reactions = [
      {
        id: "r1",
        message_id: "old-root",
        user_id: "alice",
        emoji: "👍",
        created_at: "2020-01-01T00:00:00Z",
      },
    ];
    render(pane());
    expect(await screen.findByText("Alice (@alice)")).toBeTruthy();
  });
});
