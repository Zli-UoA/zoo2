// @vitest-environment happy-dom
import { createElement, useEffect } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vite-plus/test";
import { CatchUpScreen } from "./CatchUpScreen";

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  config: { apiBaseUrl: "" },
  revision: 1,
  rangeReady: true,
  boundaryTooOld: false,
  applyRead: vi.fn(),
  navigate: vi.fn(),
  unread: [] as Array<{
    channel_id: string;
    has_unread: boolean;
    unread_count: number;
    unread_count_truncated: boolean;
    first_unread_at: string | null;
  }>,
  channels: [
    { id: "a", channel_name: "general", is_dm: false, archive_at: null as string | null },
    { id: "b", channel_name: "random", is_dm: false, archive_at: null as string | null },
  ],
  pageProps: [] as Array<{ channelId: string; readOnly?: boolean }>,
}));

vi.mock("../../hooks/useUnread", () => ({
  READ_STATE_CHANGED: "zoo-read-state-changed",
  unreadConversations: (states: Array<{ has_unread: boolean }>) =>
    states.filter((state) => state.has_unread),
  useUnread: () => ({
    states: mocks.unread,
    unreadStates: mocks.unread.filter((s) => s.has_unread),
    isReady: true,
    revision: mocks.revision,
    isError: false,
    refresh: vi.fn(),
    applyRead: mocks.applyRead,
  }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me" }),
}));
vi.mock("../../hooks/useRealtime", () => ({
  useActiveChannels: () => ({
    data: mocks.channels,
  }),
  useUsers: () => ({ data: [] }),
  useMessageReactions: () => ({ data: [] }),
}));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: mocks.config, channelUsers: {} }),
}));
vi.mock("../../lib/api/client", () => ({
  createConfiguredApiClient: () => ({ post: mocks.post, get: mocks.get }),
}));
vi.mock("@tanstack/react-db", () => ({ useLiveQuery: () => ({ data: [] }) }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => mocks.navigate }));
vi.mock("../i18n/LocaleProvider", () => ({ useLocale: () => ({ locale: "ja" }) }));
vi.mock("./dm", () => ({ buildDmLabels: () => new Map() }));
vi.mock("./CatchUpMessagePage", () => ({
  CatchUpMessagePage: ({
    channelId,
    readOnly,
    onBoundaryMessage,
    onRangeReady,
  }: {
    channelId: string;
    readOnly?: boolean;
    onBoundaryMessage: (id: string | null, at: number | null) => void;
    onRangeReady?: (ready: boolean, exhausted: boolean, boundaryTooOld: boolean) => void;
  }) => {
    mocks.pageProps.push({ channelId, readOnly });
    useEffect(() => {
      onBoundaryMessage("latest-in-page", 1);
      onRangeReady?.(mocks.rangeReady, false, mocks.boundaryTooOld);
    }, [onBoundaryMessage, onRangeReady]);
    return createElement("div", null, "message page");
  },
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, string | number>) => {
      const labels: Record<string, string> = {
        "catchUp.title": "未読キャッチアップ",
        "catchUp.close": "終了",
        "catchUp.count_one": `未読 ${options?.count} 件`,
        "catchUp.count_other": `未読 ${options?.count} 件`,
        "catchUp.progress": `${options?.current} / ${options?.total}`,
        "catchUp.markRead": "既読にする",
        "catchUp.keepUnread": "未読のまま次へ",
        "catchUp.readError": "既読にできませんでした",
        "catchUp.complete": "キャッチアップ完了",
        "catchUp.summary": `既読 ${options?.read} 件・未読のまま ${options?.skipped} 件`,
        "catchUp.empty": "未読の会話はありません",
      };
      return labels[key] ?? key;
    },
  }),
}));

const unreadItem = (channelId: string) => ({
  channel_id: channelId,
  has_unread: true,
  unread_count: 2,
  unread_count_truncated: false,
  first_unread_at: "2026-01-01T00:00:00Z",
});

afterEach(() => {
  cleanup();
  mocks.get.mockReset();
  mocks.post.mockReset();
  mocks.applyRead.mockReset();
  mocks.navigate.mockReset();
  mocks.unread = [];
  mocks.channels = [
    { id: "a", channel_name: "general", is_dm: false, archive_at: null },
    { id: "b", channel_name: "random", is_dm: false, archive_at: null },
  ];
  mocks.pageProps = [];
  mocks.revision = 1;
  mocks.rangeReady = true;
  mocks.boundaryTooOld = false;
});

async function clickMarkRead() {
  const button = await screen.findByRole("button", { name: "既読にする" });
  await waitFor(() => expect(button.hasAttribute("disabled")).toBe(false));
  await act(async () => {
    fireEvent.click(button);
  });
}

it("does not write on open, freezes the queue and marks read only on demand", async () => {
  mocks.unread = [unreadItem("a"), unreadItem("b")];
  mocks.get.mockResolvedValue(mocks.unread);
  mocks.post.mockResolvedValue({ data: { channel_id: "a", has_unread: false } });
  const view = render(createElement(CatchUpScreen));
  expect(await screen.findByText("1 / 2")).toBeTruthy();
  expect(mocks.get).toHaveBeenCalledWith("/api/channel-read-states?details=1");
  expect(mocks.post).not.toHaveBeenCalled();

  // A third conversation arriving mid-flight must not shift the page numbers.
  await act(async () => {
    mocks.unread = [unreadItem("a"), unreadItem("b"), unreadItem("c")];
    view.rerender(createElement(CatchUpScreen));
  });
  expect(screen.getByText("1 / 2")).toBeTruthy();

  await clickMarkRead();
  expect(mocks.post).toHaveBeenCalledWith("/api/channels/a/read-state", {
    message_id: "latest-in-page",
  });
  expect(mocks.applyRead).toHaveBeenCalledWith({ channel_id: "a", has_unread: false });
  expect(screen.getByText("2 / 2")).toBeTruthy();

  await act(async () => {
    fireEvent.click(screen.getByText("未読のまま次へ"));
  });
  expect(screen.getByText("キャッチアップ完了")).toBeTruthy();
  expect(screen.getByText("既読 1 件・未読のまま 1 件")).toBeTruthy();
  expect(mocks.post).toHaveBeenCalledTimes(1);
});

it("stays on the page and shows an error when the write fails", async () => {
  mocks.unread = [unreadItem("a")];
  mocks.get.mockResolvedValue(mocks.unread);
  mocks.post.mockRejectedValue(new Error("network"));
  render(createElement(CatchUpScreen));
  await screen.findByText("1 / 1");
  await clickMarkRead();
  expect(screen.getByText("既読にできませんでした")).toBeTruthy();
  expect(screen.getByText("1 / 1")).toBeTruthy();
  expect(mocks.applyRead).not.toHaveBeenCalled();
});

it("skips a conversation that became read in another tab", async () => {
  mocks.unread = [unreadItem("a"), unreadItem("b")];
  mocks.get.mockResolvedValue(mocks.unread);
  const view = render(createElement(CatchUpScreen));
  await screen.findByText("1 / 2");
  await act(async () => {
    mocks.unread = [{ ...unreadItem("a"), has_unread: false }, unreadItem("b")];
    mocks.revision = 2;
    view.rerender(createElement(CatchUpScreen));
  });
  expect(screen.getByText("2 / 2")).toBeTruthy();
  expect(mocks.post).not.toHaveBeenCalled();
});

it("does not skip a conversation whose shared state was not refreshed", async () => {
  // A is still unread in the shared list; B is stale-marking read there, but the
  // detailed response puts both into the queue.
  mocks.unread = [unreadItem("a"), { ...unreadItem("b"), has_unread: false }];
  mocks.get.mockResolvedValue([unreadItem("a"), unreadItem("b")]);
  mocks.post.mockResolvedValue({ data: { channel_id: "a", has_unread: false } });
  render(createElement(CatchUpScreen));
  expect(await screen.findByText("1 / 2")).toBeTruthy();
  await clickMarkRead();
  expect(screen.getByText("2 / 2")).toBeTruthy();
});

it("keeps marking read disabled while the first unread is outside the loaded range", async () => {
  mocks.unread = [unreadItem("a")];
  mocks.get.mockResolvedValue(mocks.unread);
  mocks.rangeReady = false;
  render(createElement(CatchUpScreen));
  const button = await screen.findByRole("button", { name: "既読にする" });
  expect(button.hasAttribute("disabled")).toBe(true);
  expect(mocks.post).not.toHaveBeenCalled();
});

it("blocks marking read and shows guidance when the boundary is older than the first unread", async () => {
  mocks.unread = [unreadItem("a")];
  mocks.get.mockResolvedValue(mocks.unread);
  mocks.boundaryTooOld = true;
  render(createElement(CatchUpScreen));
  expect(await screen.findByText("catchUp.unreadChanged")).toBeTruthy();
  const button = await screen.findByRole("button", { name: "既読にする" });
  expect(button.hasAttribute("disabled")).toBe(true);
  expect(mocks.post).not.toHaveBeenCalled();
});

it("shows the load error instead of the empty state when the detail fetch fails", async () => {
  mocks.get.mockRejectedValue(new Error("network"));
  render(createElement(CatchUpScreen));
  expect(await screen.findByText("catchUp.loadError")).toBeTruthy();
  expect(screen.queryByText("未読の会話はありません")).toBeNull();
});

it("shows the empty state when the detailed response has no unread conversations", async () => {
  mocks.get.mockResolvedValue([]);
  render(createElement(CatchUpScreen));
  expect(await screen.findByText("未読の会話はありません")).toBeTruthy();
});

it("passes readOnly to CatchUpMessagePage when the channel is archived", async () => {
  mocks.unread = [unreadItem("a")];
  mocks.get.mockResolvedValue(mocks.unread);
  const view = render(createElement(CatchUpScreen));
  await screen.findByText("1 / 1");
  expect(mocks.pageProps.at(-1)).toEqual({ channelId: "a", readOnly: false });

  await act(async () => {
    mocks.channels = [
      { id: "a", channel_name: "general", is_dm: false, archive_at: "2026-01-02T00:00:00Z" },
      { id: "b", channel_name: "random", is_dm: false, archive_at: null },
    ];
    view.rerender(createElement(CatchUpScreen));
  });
  expect(mocks.pageProps.at(-1)).toEqual({ channelId: "a", readOnly: true });
});
