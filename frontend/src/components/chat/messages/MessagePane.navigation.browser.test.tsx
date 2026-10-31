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
    parent_id: null,
    created_at: "2020-01-01T00:00:00.000Z",
    updated_at: null,
    deleted_at: null as string | null,
    user_name: "alice",
    display_name: "Alice",
  };
  return {
    root,
    exists: true,
    rows: [root],
    empty: [],
    closeThread: vi.fn(),
    openThread: vi.fn(),
    api: { request: vi.fn() },
  };
});
const collection = {
  get: () => (fixture.exists ? fixture.root : undefined),
  values: () => (fixture.exists ? fixture.rows : fixture.empty).values(),
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
  useUsers: () => ({ data: fixture.empty }),
  useChannelMembers: () => ({ data: fixture.empty }),
  useMessageReactions: () => ({ data: fixture.empty, isLoading: false, isError: false }),
  useMessageFeed: (_channel: string, { since }: { since: string }) => ({
    data:
      fixture.exists && !fixture.root.deleted_at && since <= fixture.root.created_at
        ? fixture.rows
        : fixture.empty,
    isLoading: false,
    isError: false,
  }),
}));
vi.mock("../../../hooks/useAttachments", () => ({
  useMessageAttachments: () => ({ data: fixture.empty }),
  useAttachmentUpload: () => vi.fn(),
}));
vi.mock("../../../hooks/useThread", () => ({
  useReplyCounts: () => new Map(),
  useThreadFeed: () => ({ data: fixture.empty, isLoading: false, isError: false }),
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
vi.mock("./MessageList", () => ({ MessageList: () => null }));
vi.mock("./ThreadPane", () => ({
  ThreadPane: ({ root }: { root: { content: string } }) => <div>{root.content}</div>,
}));
vi.mock("./ReactionPicker", () => ({ ReactionPicker: () => null }));

beforeEach(() => {
  fixture.exists = true;
  fixture.root.deleted_at = null;
  fixture.closeThread.mockClear();
});
describe("thread URLs outside the initial message window", () => {
  it("waits for an existing older root instead of closing its thread", async () => {
    render(
      <LocaleProvider>
        <MessagePane channelId="general" isMember={false} />
      </LocaleProvider>,
    );
    await screen.findByText("Old thread");
    expect(fixture.closeThread).not.toHaveBeenCalled();
  });
  it.each(["missing", "deleted"])("still closes a thread when its root is %s", async (kind) => {
    fixture.exists = kind !== "missing";
    if (kind === "deleted") fixture.root.deleted_at = "2020-01-02T00:00:00.000Z";
    render(
      <LocaleProvider>
        <MessagePane channelId="general" isMember={false} />
      </LocaleProvider>,
    );
    await waitFor(() => expect(fixture.closeThread).toHaveBeenCalled());
  });
});
