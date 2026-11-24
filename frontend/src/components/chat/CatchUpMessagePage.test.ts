// @vitest-environment happy-dom
import { createElement, type ReactNode } from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import type { MessageReactionRow } from "../../lib/electric/types";
import { CatchUpMessagePage } from "./CatchUpMessagePage";

const mocks = vi.hoisted(() => ({
  messages: [] as Array<{ id: string; user_id: string; created_at: string }>,
  feedLoading: false,
  feedError: false,
  isOnline: true,
  reactions: [] as MessageReactionRow[],
  reactionsLoading: false,
  reactionsError: false,
  insertReaction: vi.fn(),
  deleteReaction: vi.fn(),
  loadOlder: vi.fn(),
  hasOlder: false,
  providerProps: [] as Array<{ encrypted?: boolean }>,
  rangeReady: [] as boolean[],
  exhausted: [] as boolean[],
  boundaryTooOld: [] as boolean[],
  messageList: vi.fn(),
  pickReaction: null as ((emoji: string) => void) | null,
}));

const message = (id: string, created_at: string) => ({
  id,
  user_id: "other",
  created_at,
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("sonner", () => ({
  toast: { error: vi.fn() },
}));
vi.mock("../../hooks/useRealtime", () => ({
  useMessageFeed: () => ({
    data: mocks.messages,
    isLoading: mocks.feedLoading,
    isError: mocks.feedError,
    isInitialSyncLoading: false,
    hasOlder: mocks.hasOlder,
    isLoadingOlder: false,
    loadOlder: mocks.loadOlder,
  }),
  useUsers: () => ({ data: [], isLoading: false }),
  useMessageReactions: () => ({
    data: mocks.reactions,
    isLoading: mocks.reactionsLoading,
    isError: mocks.reactionsError,
  }),
}));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({
    messages: () => ({
      get: (id: string) => {
        const row = mocks.messages.find((item) => item.id === id);
        return row
          ? { ...row, parent_id: null, deleted_at: null, thread_archive_at: null }
          : undefined;
      },
    }),
    reactions: () => ({
      values: () => mocks.reactions.values(),
      insert: mocks.insertReaction,
      delete: mocks.deleteReaction,
    }),
  }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me" }),
}));
vi.mock("../../hooks/useMessageDecryptions", () => ({ useMessageDecryptions: () => new Map() }));
vi.mock("../../hooks/useKitchenCatalog", () => ({ useKitchenCatalog: () => ({ catalog: null }) }));
vi.mock("../../hooks/useAttachments", () => ({ useMessageAttachments: () => ({ data: [] }) }));
vi.mock("../../lib/attachments", () => ({ groupAttachmentsByMessage: () => new Map() }));
vi.mock("../../lib/offline/offline-context", () => ({
  useOfflineStatus: () => ({ isOnline: mocks.isOnline }),
}));
vi.mock("../../hooks/useEncryptedDisplay", () => ({
  useEncryptedDisplay: ({
    encrypted,
    channelName,
  }: {
    encrypted?: boolean;
    channelName?: string;
  }) => ({
    treatAsEncrypted: !!encrypted || !!channelName?.startsWith("edm:"),
    downgraded: false,
    checkingEncryption: false,
  }),
}));
vi.mock("../../lib/e2ee/messageDecryption", () => ({
  firstEncryptedAt: () => null,
  useEncryptedChannel: () => false,
  withUnencryptedRows: (_rows: unknown, decryptions: unknown) => decryptions,
  MessageDecryptionsProvider: ({
    children,
    encrypted,
  }: {
    children: ReactNode;
    encrypted?: boolean;
  }) => {
    mocks.providerProps.push({ encrypted });
    return children;
  },
}));
vi.mock("./messages/MessageList", () => ({
  MessageList: (props: Record<string, unknown>) => {
    mocks.messageList(props);
    return createElement("div", null, "messages");
  },
}));
vi.mock("./messages/ReactionPicker", () => ({
  ReactionPicker: ({ open, onSelect }: { open: boolean; onSelect: (emoji: string) => void }) => {
    mocks.pickReaction = open ? onSelect : null;
    return open ? createElement("div", null, "reaction-picker") : null;
  },
}));

afterEach(() => {
  cleanup();
  mocks.messages = [];
  mocks.feedLoading = false;
  mocks.feedError = false;
  mocks.isOnline = true;
  mocks.reactions = [];
  mocks.reactionsLoading = false;
  mocks.reactionsError = false;
  mocks.insertReaction.mockReset();
  mocks.deleteReaction.mockReset();
  mocks.hasOlder = false;
  mocks.providerProps = [];
  mocks.rangeReady = [];
  mocks.exhausted = [];
  mocks.boundaryTooOld = [];
  mocks.loadOlder.mockReset();
  mocks.messageList.mockClear();
  mocks.pickReaction = null;
});

it("passes the viewer and reaction handlers without enabling edit, delete or thread actions", () => {
  render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: null,
      onBoundaryMessage: vi.fn(),
    }),
  );
  expect(mocks.messageList).toHaveBeenCalledWith(
    expect.objectContaining({
      currentUserId: null,
      viewerId: "me",
      canReact: true,
      onReaction: expect.any(Function),
      onPickReaction: expect.any(Function),
    }),
  );
  expect(mocks.messageList.mock.calls.at(-1)?.[0].onOpenThread).toBeUndefined();
});

it.each([
  { label: "readOnly", props: { readOnly: true }, setup: () => {} },
  {
    label: "offline",
    props: {},
    setup: () => {
      mocks.isOnline = false;
    },
  },
  {
    label: "messages loading",
    props: {},
    setup: () => {
      mocks.feedLoading = true;
    },
  },
  {
    label: "messages error",
    props: {},
    setup: () => {
      mocks.feedError = true;
    },
  },
  {
    label: "reactions loading",
    props: {},
    setup: () => {
      mocks.reactionsLoading = true;
    },
  },
  {
    label: "reactions error",
    props: {},
    setup: () => {
      mocks.reactionsError = true;
    },
  },
])("disables reacting when $label", ({ props, setup }) => {
  setup();
  render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: null,
      onBoundaryMessage: vi.fn(),
      ...props,
    }),
  );
  expect(mocks.messageList).toHaveBeenCalledWith(expect.objectContaining({ canReact: false }));
  if (mocks.reactionsError) {
    expect(screen.getByText("reactions.fetchError")).toBeTruthy();
  }
});

it("re-enables reacting once the message feed finishes loading", () => {
  mocks.feedLoading = true;
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z")];
  const onBoundaryMessage = vi.fn();

  const view = render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: null,
      onBoundaryMessage,
    }),
  );
  expect(mocks.messageList.mock.calls.at(-1)?.[0]).toEqual(
    expect.objectContaining({ canReact: false }),
  );

  mocks.feedLoading = false;
  view.rerender(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: null,
      onBoundaryMessage,
    }),
  );
  expect(mocks.messageList.mock.calls.at(-1)?.[0]).toEqual(
    expect.objectContaining({ canReact: true }),
  );
});

it("adds and toggles reactions via ReactionPicker without moving the read boundary", async () => {
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z")];
  mocks.insertReaction.mockReturnValue({ when: () => Promise.resolve() });
  mocks.deleteReaction.mockReturnValue({ when: () => Promise.resolve() });
  const onBoundaryMessage = vi.fn();

  render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: null,
      onBoundaryMessage,
    }),
  );
  expect(onBoundaryMessage).toHaveBeenCalledTimes(1);

  const listProps = mocks.messageList.mock.calls.at(-1)?.[0] as {
    onPickReaction: (messageId: string) => void;
    onReaction: (messageId: string, emoji: string) => void;
  };
  await act(async () => {
    listProps.onPickReaction("m1");
  });
  expect(screen.getByText("reaction-picker")).toBeTruthy();

  await act(async () => {
    mocks.pickReaction?.("🎉");
  });
  expect(mocks.insertReaction).toHaveBeenCalledWith(
    expect.objectContaining({ message_id: "m1", user_id: "me", emoji: "🎉" }),
  );
  expect(onBoundaryMessage).toHaveBeenCalledTimes(1);

  mocks.reactions = [
    {
      id: "r-own",
      message_id: "m1",
      user_id: "me",
      emoji: "👍",
      created_at: "2026-01-01T10:01:00Z",
    },
  ];
  await act(async () => {
    listProps.onReaction("m1", "👍");
  });
  expect(mocks.deleteReaction).toHaveBeenCalledWith("r-own");
  expect(onBoundaryMessage).toHaveBeenCalledTimes(1);
});

it("keeps the boundary at the newest message shown when the page opened", async () => {
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z"), message("m2", "2026-01-01T10:05:00Z")];
  const onBoundaryMessage = vi.fn();
  const view = render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: null,
      onBoundaryMessage,
    }),
  );
  expect(onBoundaryMessage).toHaveBeenCalledTimes(1);
  expect(onBoundaryMessage).toHaveBeenCalledWith("m2", expect.any(Number));

  // A message arriving while reading must not move the read boundary.
  await act(async () => {
    mocks.messages = [...mocks.messages, message("m3", "2026-01-01T10:10:00Z")];
    view.rerender(
      createElement(CatchUpMessagePage, {
        channelId: "c1",
        firstUnreadAt: null,
        onBoundaryMessage,
      }),
    );
  });
  expect(onBoundaryMessage).toHaveBeenCalledTimes(1);
  expect(onBoundaryMessage).toHaveBeenCalledWith("m2", expect.any(Number));
});

it("moves the boundary to the newest surviving message if it is deleted", async () => {
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z"), message("m2", "2026-01-01T10:05:00Z")];
  const onBoundaryMessage = vi.fn();
  const view = render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: null,
      onBoundaryMessage,
    }),
  );
  const firstAt = onBoundaryMessage.mock.calls[0][1] as number;
  await act(async () => {
    mocks.messages = [message("m1", "2026-01-01T10:00:00Z")];
    view.rerender(
      createElement(CatchUpMessagePage, {
        channelId: "c1",
        firstUnreadAt: null,
        onBoundaryMessage,
      }),
    );
  });
  expect(onBoundaryMessage).toHaveBeenLastCalledWith("m1", firstAt);
});

it("keeps encrypted rendering when the channel is flagged encrypted or is an edm", () => {
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z")];
  const view = render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      encrypted: true,
      firstUnreadAt: null,
      onBoundaryMessage: vi.fn(),
    }),
  );
  expect(mocks.providerProps[0].encrypted).toBe(true);
  view.unmount();

  render(
    createElement(CatchUpMessagePage, {
      channelId: "c2",
      channelName: "edm:alice:bob",
      firstUnreadAt: null,
      onBoundaryMessage: vi.fn(),
    }),
  );
  expect(mocks.providerProps.at(-1)?.encrypted).toBe(true);
});

it("does not allow marking read before the oldest unread is inside the range", () => {
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z")];
  render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: "2026-01-01T09:00:00Z",
      onBoundaryMessage: vi.fn(),
      onRangeReady: (ready, exhausted, boundaryTooOld) => {
        mocks.rangeReady.push(ready);
        mocks.exhausted.push(exhausted);
        mocks.boundaryTooOld.push(boundaryTooOld);
      },
    }),
  );
  expect(mocks.rangeReady.at(-1)).toBe(false);
  expect(mocks.exhausted.at(-1)).toBe(true);

  // The first unread is newer than every loaded message: not ready either.
  mocks.rangeReady = [];
  render(
    createElement(CatchUpMessagePage, {
      channelId: "c2",
      firstUnreadAt: "2026-01-01T11:00:00Z",
      onBoundaryMessage: vi.fn(),
      onRangeReady: (ready, exhausted, boundaryTooOld) => {
        mocks.rangeReady.push(ready);
        mocks.exhausted.push(exhausted);
        mocks.boundaryTooOld.push(boundaryTooOld);
      },
    }),
  );
  expect(mocks.rangeReady.at(-1)).toBe(false);
  expect(mocks.exhausted.at(-1)).toBe(false);
  expect(mocks.boundaryTooOld.at(-1)).toBe(true);

  // Both boundaries contain the first unread.
  mocks.rangeReady = [];
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z"), message("m2", "2026-01-01T12:00:00Z")];
  render(
    createElement(CatchUpMessagePage, {
      channelId: "c3",
      firstUnreadAt: "2026-01-01T11:00:00Z",
      onBoundaryMessage: vi.fn(),
      onRangeReady: (ready, exhausted, boundaryTooOld) => {
        mocks.rangeReady.push(ready);
        mocks.exhausted.push(exhausted);
        mocks.boundaryTooOld.push(boundaryTooOld);
      },
    }),
  );
  expect(mocks.rangeReady.at(-1)).toBe(true);
  expect(mocks.boundaryTooOld.at(-1)).toBe(false);
});

it("loads older pages until the oldest unread timestamp is inside the range", async () => {
  mocks.messages = [message("m1", "2026-01-01T10:00:00Z")];
  mocks.hasOlder = true;
  render(
    createElement(CatchUpMessagePage, {
      channelId: "c1",
      firstUnreadAt: "2026-01-01T09:00:00Z",
      onBoundaryMessage: vi.fn(),
    }),
  );
  expect(mocks.loadOlder).toHaveBeenCalledTimes(1);
});
