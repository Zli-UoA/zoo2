// @vitest-environment happy-dom
import { createElement, type ReactNode } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { CatchUpMessagePage } from "./CatchUpMessagePage";

const mocks = vi.hoisted(() => ({
  messages: [] as Array<{ id: string; user_id: string; created_at: string }>,
  loadOlder: vi.fn(),
  hasOlder: false,
  providerProps: [] as Array<{ encrypted?: boolean }>,
  rangeReady: [] as boolean[],
  exhausted: [] as boolean[],
  boundaryTooOld: [] as boolean[],
}));

const message = (id: string, created_at: string) => ({
  id,
  user_id: "other",
  created_at,
});

vi.mock("../../hooks/useRealtime", () => ({
  useMessageFeed: () => ({
    data: mocks.messages,
    isLoading: false,
    isError: false,
    isInitialSyncLoading: false,
    hasOlder: mocks.hasOlder,
    isLoadingOlder: false,
    loadOlder: mocks.loadOlder,
  }),
  useUsers: () => ({ data: [] }),
  useMessageReactions: () => ({ data: [] }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me" }),
}));
vi.mock("../../hooks/useMessageDecryptions", () => ({ useMessageDecryptions: () => new Map() }));
vi.mock("../../hooks/useKitchenCatalog", () => ({ useKitchenCatalog: () => ({ catalog: null }) }));
vi.mock("../../hooks/useAttachments", () => ({ useMessageAttachments: () => ({ data: [] }) }));
vi.mock("../../lib/attachments", () => ({ groupAttachmentsByMessage: () => new Map() }));
vi.mock("../../lib/emoji/reactions", () => ({
  groupReactions: () => new Map(),
  reactionUserLabel: (user: { user_name: string }) => user.user_name,
}));
vi.mock("../../lib/offline/offline-context", () => ({
  useOfflineStatus: () => ({ isOnline: true }),
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
  MessageList: () => createElement("div", null, "messages"),
}));

afterEach(() => {
  cleanup();
  mocks.messages = [];
  mocks.hasOlder = false;
  mocks.providerProps = [];
  mocks.rangeReady = [];
  mocks.exhausted = [];
  mocks.boundaryTooOld = [];
  mocks.loadOlder.mockReset();
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
