import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { LOCALE_STORAGE_KEY } from "../../../lib/i18n/locale";
import { ApiError } from "../../../lib/api/client";
import { MessagePane } from "./MessagePane";

vi.mock("sonner", () => ({ toast: { error: vi.fn(), info: vi.fn() } }));

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
    rootError: false,
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
    /** useWriteRunner の失敗を差し替えてトーストの文言を検証する。 */
    writeError: null as string | null,
    writeErrorCause: null as unknown,
    setWriteError: vi.fn(),
    /** `useThreadMembership` の取り直し（`retry`）。参加が必要な 403 のあとに呼ばれる。 */
    retryMembership: vi.fn(),
    /** `useE2ee` の値。既定は暗号ストアなし（暗号化チャンネルの送信を試すテストだけ差し替える）。 */
    e2ee: {} as Record<string, unknown>,
    /** 暗号化チャンネルの送信（`sendEncryptedMessage`）。 */
    sendEncrypted: vi.fn(),
    /** メッセージのコレクションに入っている行（`get` で返す）。 */
    collectionRows: [] as { id: string; parent_id: string | null; deleted_at: string | null }[],
    /** モックの ReactionPicker が受け取った最新の選択。 */
    pickReaction: null as ((emoji: string) => void) | null,
  };
});
const collection = {
  insert: fixture.insert,
  get: (id: string) => fixture.collectionRows.find((row) => row.id === id),
  values: () => fixture.empty.values(),
};
const collectionsBundle = {
  messages: () => collection,
  reactions: () => collection,
  attachments: () => collection,
  channelUsers: {},
  config: {},
};
vi.mock("../../../lib/electric/collections-context", () => ({
  useCollections: () => collectionsBundle,
  useOptionalCollections: () => collectionsBundle,
}));
vi.mock("../../../lib/api/client", async (original) => ({
  ...(await original<typeof import("../../../lib/api/client")>()),
  createConfiguredApiClient: () => fixture.api,
}));
vi.mock("../../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "alice", isAdmin: false }),
}));
vi.mock("../../../hooks/useThreadMembership", async (original) => ({
  ...(await original<typeof import("../../../hooks/useThreadMembership")>()),
  useThreadMembership: () => ({
    data: null,
    pending: false,
    error: false,
    change: vi.fn(),
    retry: fixture.retryMembership,
  }),
}));
vi.mock("../../../lib/e2ee/e2ee-context", async (original) => ({
  ...(await original<typeof import("../../../lib/e2ee/e2ee-context")>()),
  useE2ee: () => fixture.e2ee,
}));
vi.mock("../../../lib/e2ee/channelSend", async (original) => ({
  ...(await original<typeof import("../../../lib/e2ee/channelSend")>()),
  sendEncryptedMessage: (...args: unknown[]) => fixture.sendEncrypted(...args),
}));
vi.mock("../../../hooks/useKitchenCatalog", () => ({
  useKitchenCatalog: () => ({ catalog: null }),
}));
vi.mock("../../../hooks/useRealtime", () => ({
  useUsers: () => ({ data: fixture.users, isLoading: false }),
  useActiveChannels: () => ({
    data: fixture.empty,
    isLoading: false,
    isInitialSyncLoading: false,
  }),
  useMyChannelMemberships: () => ({
    memberships: fixture.empty,
    isLoading: false,
  }),
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
vi.mock("../../../hooks/useAttachments", async (original) => ({
  ...(await original<typeof import("../../../hooks/useAttachments")>()),
  useMessageAttachments: () => ({ data: fixture.empty }),
  useAttachmentUpload: () => vi.fn(),
  useEncryptedAttachmentUpload: () => vi.fn(),
}));
vi.mock("../../../hooks/useThread", () => ({
  useMessageById: () => ({ data: fixture.targetRows, isLoading: false }),
  useThreadRoot: () => ({
    data: fixture.rootRows,
    isLoading: fixture.rootLoading,
    isError: fixture.rootError,
  }),
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
  useWriteRunner: () => ({
    error: fixture.writeError,
    errorCause: fixture.writeErrorCause,
    setError: fixture.setWriteError,
    run: async (action: () => unknown) => action(),
    isPending: false,
  }),
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
    currentUserId,
    viewerId,
  }: {
    reactions: Map<string, { userNames?: Record<string, string> }[]>;
    currentUserId: string | null;
    viewerId?: string | null;
  }) => (
    <div
      data-testid="message-list-mock"
      data-current-user-id={currentUserId ?? ""}
      data-viewer-id={viewerId ?? ""}
    >
      {reactions.get("old-root")?.[0]?.userNames?.alice}
    </div>
  ),
}));
vi.mock("./ThreadPane", () => ({
  ThreadPane: ({
    root,
    replies,
    targetMessageId,
    readOnly,
    isMember,
    onSendReply,
    onPickReaction,
    onRequestDelete,
    currentUserId,
    viewerId,
    mentionUserId,
  }: {
    root: { content: string };
    replies: { id: string; content: string }[];
    targetMessageId?: string;
    readOnly?: boolean;
    isMember?: boolean;
    onSendReply: (content: string) => Promise<boolean>;
    onPickReaction?: (id: string) => void;
    onRequestDelete?: (id: string) => void;
    currentUserId: string | null;
    viewerId?: string | null;
    mentionUserId?: string | null;
  }) => (
    <div
      data-testid="thread-pane-mock"
      data-target-message-id={targetMessageId ?? ""}
      data-current-user-id={currentUserId ?? ""}
      data-viewer-id={viewerId ?? ""}
      data-mention-user-id={mentionUserId ?? ""}
    >
      {root.content}
      {replies.map((reply) => (
        <p key={reply.id}>
          {reply.content}
          <button onClick={() => onPickReaction?.(reply.id)}>React to {reply.id}</button>
          <button onClick={() => onRequestDelete?.(reply.id)}>Delete {reply.id}</button>
        </p>
      ))}
      {!readOnly ? (
        <button disabled={!isMember} onClick={() => void onSendReply("Continued reply")}>
          Reply to thread
        </button>
      ) : null}
    </div>
  ),
}));
vi.mock("./ReactionPicker", () => ({
  ReactionPicker: ({ open, onSelect }: { open: boolean; onSelect: (emoji: string) => void }) => {
    fixture.pickReaction = open ? onSelect : null;
    return open ? <div>Reaction picker</div> : null;
  },
}));

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
  fixture.rootError = false;
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
  fixture.writeError = null;
  fixture.writeErrorCause = null;
  fixture.setWriteError.mockClear();
  fixture.retryMembership.mockClear();
  fixture.e2ee = {};
  fixture.sendEncrypted.mockReset();
  fixture.collectionRows = [];
  fixture.pickReaction = null;
  vi.mocked(toast.error).mockClear();
  vi.mocked(toast.info).mockClear();
});

describe("links and threads whose rows are older than the loaded range", () => {
  it.each([
    { isMember: true, readOnly: false, actionId: "alice", mentionId: "alice" },
    { isMember: true, readOnly: true, actionId: "", mentionId: "alice" },
    { isMember: false, readOnly: false, actionId: "alice", mentionId: "" },
  ])(
    "keeps membership-based mentions separate from participant identity: $isMember / $readOnly",
    async ({ isMember, readOnly, actionId, mentionId }) => {
      render(
        <LocaleProvider>
          <MessagePane channelId="general" isMember={isMember} readOnly={readOnly} />
        </LocaleProvider>,
      );
      const list = await screen.findByTestId("message-list-mock");
      expect(list.dataset.currentUserId).toBe(actionId);
      expect(list.dataset.viewerId).toBe(mentionId);
      const thread = await screen.findByTestId("thread-pane-mock");
      expect(thread.dataset.currentUserId).toBe(actionId);
      expect(thread.dataset.viewerId).toBe("alice");
      expect(thread.dataset.mentionUserId).toBe(mentionId);
    },
  );

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
  it("passes the viewer id to an archived thread while keeping the action id null", async () => {
    fixture.root.thread_archive_at = new Date().toISOString();
    render(
      <LocaleProvider>
        <MessagePane channelId="general" isMember />
      </LocaleProvider>,
    );
    const threadPane = await screen.findByTestId("thread-pane-mock");
    // 編集・削除などは止めたまま、メンバーのダイアログの「あなた」の印には閲覧者の ID を使う。
    expect(threadPane.dataset.currentUserId).toBe("");
    expect(threadPane.dataset.viewerId).toBe("alice");
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

  it("passes targetMessageId to ThreadPane only for replies in the active thread", async () => {
    const { rerender } = render(pane("old-root"));
    const paneEl = await screen.findByTestId("thread-pane-mock");
    expect(paneEl.getAttribute("data-target-message-id")).toBe("");

    fixture.targetRows = [{ ...fixture.root, id: "unloaded-reply", parent_id: "old-root" }];
    rerender(pane("unloaded-reply"));
    await waitFor(() => {
      expect(screen.getByTestId("thread-pane-mock").getAttribute("data-target-message-id")).toBe(
        "unloaded-reply",
      );
    });
  });

  it("passes targetMessageId to ThreadPane when opening a surviving reply under a deleted root", async () => {
    const reply = {
      ...fixture.root,
      id: "surviving-reply",
      content: "Surviving reply",
      parent_id: fixture.root.id,
      deleted_at: null,
    };
    fixture.root.deleted_at = "2020-01-02 00:00:00.000";
    fixture.replies = [reply];
    fixture.targetRows = [reply];
    render(pane("surviving-reply"));
    const paneEl = await screen.findByTestId("thread-pane-mock");
    expect(paneEl.getAttribute("data-target-message-id")).toBe("surviving-reply");
    expect(fixture.closeThread).not.toHaveBeenCalled();
  });

  it("does not close a thread whose root is still being fetched", async () => {
    fixture.rootRows = [];
    fixture.rootLoading = true;
    const { rerender } = render(pane());
    expect(fixture.closeThread).not.toHaveBeenCalled();
    // 取得が終わっても、その行がなければ、(存在しないので) 閉じる
    fixture.rootLoading = false;
    fixture.rootError = false;
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

  it("closes the reaction picker of a reply and does not write once its root is deleted", async () => {
    const reply = {
      ...fixture.root,
      id: "reply-1",
      content: "Surviving reply",
      parent_id: fixture.root.id,
    };
    fixture.replies = [reply];
    fixture.targetRows = [reply];
    fixture.collectionRows = [fixture.root, reply];
    const { rerender } = render(pane("reply-1"));
    await userEvent.click(await screen.findByRole("button", { name: "React to reply-1" }));
    await screen.findByText("Reaction picker");
    const pick = fixture.pickReaction!;
    // 選んだ後に親が削除された（スレッドは返信を読むだけで残る）。
    fixture.root.deleted_at = "2020-01-02 00:00:00.000";
    rerender(pane("reply-1"));
    await waitFor(() => expect(screen.queryByText("Reaction picker")).toBeNull());
    expect(fixture.closeThread).not.toHaveBeenCalled();
    pick("👍");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(fixture.insert).not.toHaveBeenCalled();
  });

  it("closes the delete confirmation of a reply once its root is deleted", async () => {
    const reply = {
      ...fixture.root,
      id: "reply-1",
      content: "Surviving reply",
      parent_id: fixture.root.id,
    };
    fixture.replies = [reply];
    fixture.targetRows = [reply];
    fixture.collectionRows = [fixture.root, reply];
    const { rerender } = render(pane("reply-1"));
    await userEvent.click(await screen.findByRole("button", { name: "Delete reply-1" }));
    await screen.findByRole("alertdialog");
    fixture.root.deleted_at = "2020-01-02 00:00:00.000";
    rerender(pane("reply-1"));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(fixture.closeThread).not.toHaveBeenCalled();
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

it("shows a retryable pane without closing when its root request fails", async () => {
  fixture.rootRows = [];
  fixture.rootError = true;
  render(pane());
  expect(fixture.closeThread).not.toHaveBeenCalled();
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(screen.getByRole("button", { name: "再試行" })).toBeTruthy();
  screen.getByRole("button", { name: "スレッドを閉じる" }).click();
  expect(fixture.closeThread).toHaveBeenCalledTimes(1);
});

it("shows a localized description for an offline write failure", async () => {
  const previousLocale = localStorage.getItem(LOCALE_STORAGE_KEY);
  localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  try {
    fixture.writeError = "Failed to fetch";
    fixture.writeErrorCause = new TypeError("Failed to fetch");
    render(pane());
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith("操作に失敗しました", {
        description: "サーバーに接続できませんでした。接続を確認してもう一度お試しください。",
        id: "message-write-error",
      }),
    );
    expect(fixture.setWriteError).toHaveBeenCalledWith(null);
  } finally {
    if (previousLocale === null) localStorage.removeItem(LOCALE_STORAGE_KEY);
    else localStorage.setItem(LOCALE_STORAGE_KEY, previousLocale);
  }
});

describe("a reply that the server refuses until the thread is joined", () => {
  async function failWith(cause: unknown) {
    fixture.writeError = "Forbidden";
    fixture.writeErrorCause = cause;
    render(pane());
    await waitFor(() => expect(fixture.setWriteError).toHaveBeenCalledWith(null));
  }

  it.each([
    ["the code", { error: "any text", code: "THREAD_JOIN_REQUIRED" }],
    [
      "the old message, from a server that sends no code",
      { error: "Join the thread before replying" },
    ],
  ])("refreshes the membership and asks for a join (%s)", async (_label, body) => {
    await failWith(new ApiError(403, "forbidden", body));
    expect(fixture.retryMembership).toHaveBeenCalled();
    expect(toast.info).toHaveBeenCalledWith(
      expect.stringContaining("参加"),
      expect.objectContaining({ id: "thread-join-required" }),
    );
    expect(toast.error).not.toHaveBeenCalled();
  });

  it.each([
    ["another code", { error: "Join the thread before replying", code: "OTHER" }],
    [
      "a posting restriction",
      { error: "Only channel managers may post here", code: "POSTING_RESTRICTED" },
    ],
  ])("keeps the generic error for a 403 that a join does not cure (%s)", async (_label, body) => {
    await failWith(new ApiError(403, "forbidden", body));
    expect(fixture.retryMembership).not.toHaveBeenCalled();
    expect(toast.info).not.toHaveBeenCalled();
    expect(toast.error).toHaveBeenCalled();
  });

  it("answers the same 403 of an encrypted reply the same way", async () => {
    fixture.e2ee = {
      worker: { request: vi.fn().mockResolvedValue(undefined) },
      deviceId: "device",
      requestKeyBackup: vi.fn(),
    };
    fixture.sendEncrypted.mockRejectedValue(
      new ApiError(403, "forbidden", { error: "any text", code: "THREAD_JOIN_REQUIRED" }),
    );
    render(
      <LocaleProvider>
        <MessagePane channelId="general" isMember encrypted />
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Reply to thread" }));
    await waitFor(() => expect(fixture.retryMembership).toHaveBeenCalled());
    expect(toast.info).toHaveBeenCalledWith(
      expect.stringContaining("参加"),
      expect.objectContaining({ id: "thread-join-required" }),
    );
    expect(toast.error).not.toHaveBeenCalled();
  });

  it("keeps the generic error for an encrypted reply refused for another reason", async () => {
    fixture.e2ee = {
      worker: { request: vi.fn().mockResolvedValue(undefined) },
      deviceId: "device",
      requestKeyBackup: vi.fn(),
    };
    fixture.sendEncrypted.mockRejectedValue(
      new ApiError(403, "forbidden", {
        error: "Only channel managers may post here",
        code: "POSTING_RESTRICTED",
      }),
    );
    render(
      <LocaleProvider>
        <MessagePane channelId="general" isMember encrypted />
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Reply to thread" }));
    await waitFor(() => expect(toast.error).toHaveBeenCalled());
    expect(fixture.retryMembership).not.toHaveBeenCalled();
    expect(toast.info).not.toHaveBeenCalled();
  });
});
