import "../../styles.css";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { ApiError } from "../../lib/api/client";
import { usePinnedChannels } from "../../hooks/usePinnedChannels";
import { ChannelSidebar } from "./ChannelSidebar";
import type { ChannelRow } from "../../lib/electric/types";
import type { ThreadSummary } from "../../hooks/useThreadIndex";

const fixture = vi.hoisted(() => ({
  memberships: [{ user_id: "other", channel_id: "general" }] as Array<{
    user_id: string;
    channel_id: string;
    position?: number;
  }>,
  threads: [] as ThreadSummary[],
  navigate: vi.fn(),
  config: {},
  userPinnedChannels: () => ({}),
  get: vi.fn(),
  post: vi.fn(),
  del: vi.fn(),
  patch: vi.fn(),
  selectedId: null as string | null,
  users: [] as Array<Record<string, unknown>>,
  pins: [] as Array<{
    user_id: string;
    channel_id: string;
    position: number;
    created_at: string;
    updated_at: string;
  }>,
  pinError: null as string | null,
  activeCalls: new Map<string, number>(),
  pinChannel: vi.fn(),
  unpinChannel: vi.fn(),
  reorderPins: vi.fn(),
  refresh: vi.fn(),
  dismissError: vi.fn(),
}));
vi.mock("@tanstack/react-db", () => ({
  useLiveQuery: () => ({
    data: fixture.memberships,
    isLoading: true,
    isPersistedReady: true,
    isError: false,
  }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    params,
    search,
    onClick,
    ...props
  }: import("react").AnchorHTMLAttributes<HTMLAnchorElement> & {
    to: string;
    params: { channelId: string };
    search?: { thread: string };
  }) => (
    <a
      {...props}
      href={`/channels/${params.channelId}${search ? `?thread=${search.thread}` : ""}`}
      onClick={(event) => {
        event.preventDefault();
        onClick?.(event);
        fixture.navigate({ to, params, search });
      }}
    >
      {children}
    </a>
  ),
  useNavigate: () => fixture.navigate,
}));
vi.mock("../../hooks/useThread", () => ({
  useThreadRoot: () => ({
    data: fixture.threads.filter((thread) => thread.id === fixture.selectedId),
  }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isLoading: false }),
}));
vi.mock("../../hooks/useRealtime", () => ({
  useUsers: () => ({ data: fixture.users, isLoading: false }),
}));
vi.mock("../../hooks/useUnread", () => ({
  READ_STATE_CHANGED: "zoo-read-state-changed",
  useUnread: () => ({ unreadChannelIds: new Set() }),
}));
vi.mock("../../hooks/useChannelLeaveDates", () => ({ useChannelLeaveDates: () => new Map() }));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({
    channelUsers: {},
    config: fixture.config,
    userPinnedChannels: fixture.userPinnedChannels,
  }),
}));
vi.mock("../../lib/api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/api/client")>();
  return {
    ...actual,
    createConfiguredApiClient: () => ({
      get: fixture.get,
      post: fixture.post,
      delete: fixture.del,
      patch: fixture.patch,
    }),
  };
});
vi.mock("../../lib/call/call-context", () => ({
  useCall: () => ({ activeCalls: fixture.activeCalls }),
}));
vi.mock("../layout/shell-context", () => ({
  useShell: () => ({
    selectedChannelId: "general",
    threadRootId: fixture.selectedId,
    showChat: vi.fn(),
  }),
}));
vi.mock("../SyncStatus", () => ({ SyncStatus: () => null }));
vi.mock("./ChannelBrowserDialog", () => ({ ChannelBrowserDialog: () => null }));
vi.mock("./CreateChannelDialog", () => ({ CreateChannelDialog: () => null }));
vi.mock("./CreateDmDialog", () => ({ CreateDmDialog: () => null }));
vi.mock("../../hooks/usePinnedChannels", async () => {
  const actual = await vi.importActual<typeof import("../../hooks/usePinnedChannels")>(
    "../../hooks/usePinnedChannels",
  );
  return {
    ...actual,
    // 既定は操作できない差し替え。実際の Provider を使うテストでは、
    // `vi.mocked(usePinnedChannels).mockImplementation(actual.usePinnedChannels)` にする。
    usePinnedChannels: vi.fn(),
  };
});

/** 差し替え用のピン留め状態。`fixture` をそのまま読むので、テスト中に書き換えられる。 */
function mockedPinnedChannels() {
  return {
    available: true,
    pins: fixture.pins,
    version: 0,
    loading: false,
    pending: new Set<string>(),
    error: fixture.pinError,
    announcement: "",
    isPinned: (channelId: string) => fixture.pins.some((pin) => pin.channel_id === channelId),
    pinChannel: fixture.pinChannel,
    unpinChannel: fixture.unpinChannel,
    reorderPins: fixture.reorderPins,
    refresh: fixture.refresh,
    dismissError: fixture.dismissError,
  };
}

beforeEach(() => {
  fixture.memberships = [{ user_id: "other", channel_id: "general" }];
  fixture.threads = [];
  fixture.selectedId = null;
  fixture.navigate.mockClear();
  fixture.config = {};
  fixture.users = [];
  fixture.pins = [];
  fixture.pinError = null;
  fixture.activeCalls = new Map();
  fixture.pinChannel.mockReset().mockResolvedValue(undefined);
  fixture.unpinChannel.mockReset().mockResolvedValue(undefined);
  fixture.reorderPins.mockReset().mockResolvedValue(undefined);
  fixture.dismissError.mockReset();
  fixture.post.mockReset();
  fixture.del.mockReset();
  fixture.patch.mockReset();
  fixture.get.mockReset().mockImplementation(async (path: string) => ({
    items: fixture.threads.filter(
      (thread) => !!thread.thread_archive_at === path.includes("archived=true"),
    ),
    next_cursor: null,
  }));
  vi.mocked(usePinnedChannels).mockReset();
  vi.mocked(usePinnedChannels).mockImplementation(mockedPinnedChannels);
});

it("waits for the current user's membership even when other users are cached", () => {
  const channel = {
    id: "general",
    channel_name: "general",
    is_dm: false,
    archive_at: null,
  } as ChannelRow;
  const view = () => (
    <LocaleProvider>
      <ChannelSidebar channels={[channel]} loading={false} error={false} />
    </LocaleProvider>
  );
  const { container, rerender } = render(view());
  expect(container.querySelector('[data-slot="skeleton"]')).not.toBeNull();
  expect(screen.queryByText("参加中のチャンネルはありません")).toBeNull();
  fixture.memberships = [...fixture.memberships, { user_id: "me", channel_id: "general" }];
  rerender(view());
  expect(screen.getByRole("link", { name: "general" })).toBeTruthy();
  expect(container.querySelector('[data-slot="skeleton"]')).toBeNull();
});

it("never shows an archived thread in the tree, even while selected, and has no history or search controls", async () => {
  fixture.memberships = [{ user_id: "me", channel_id: "general" }];
  fixture.threads = [
    {
      id: "old",
      channel_id: "general",
      content: "Old conversation",
      thread_archive_at: "2020-01-01T00:00:00Z",
      reply_count: 3,
    },
    {
      id: "active",
      channel_id: "general",
      content: "Active conversation",
      thread_archive_at: null,
      reply_count: 1,
    },
  ];
  fixture.selectedId = "old";
  const view = () => (
    <LocaleProvider>
      <ChannelSidebar
        channels={[
          { id: "general", channel_name: "general", is_dm: false, archive_at: null } as ChannelRow,
        ]}
        loading={false}
        error={false}
      />
    </LocaleProvider>
  );
  const { rerender } = render(view());
  await screen.findByRole("link", { name: /Active conversation/ });
  expect(screen.queryByRole("link", { name: /Old conversation/ })).toBeNull();
  expect(screen.queryByRole("button", { name: /アーカイブ済みスレッド/ })).toBeNull();
  expect(screen.queryByRole("searchbox")).toBeNull();
  expect(fixture.get.mock.calls.every(([path]) => !path.includes("archived=true"))).toBe(true);
  fixture.selectedId = null;
  rerender(view());
  expect(screen.queryByRole("link", { name: /Old conversation/ })).toBeNull();
});

function pinRow(channelId: string, position: number) {
  return {
    user_id: "me",
    channel_id: channelId,
    position,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: "2026-01-01 00:00:00.000",
  };
}

const generalChannel = {
  id: "general",
  channel_name: "general",
  is_dm: false,
  archive_at: null,
} as ChannelRow;
const designChannel = {
  id: "design",
  channel_name: "design",
  is_dm: false,
  archive_at: null,
} as ChannelRow;
const archivedChannel = {
  id: "old-announcements",
  channel_name: "old-announcements",
  is_dm: false,
  archive_at: "2026-01-02 00:00:00.000",
} as ChannelRow;
const dmChannel = {
  id: "dm-sidebar",
  channel_name: "dm:user-alice:user-bob",
  is_dm: true,
  archive_at: null,
} as ChannelRow;

it("stretches channel and DM links across the full row so the empty right side opens them", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: dmChannel.id },
  ];
  fixture.users = [
    {
      id: "user-alice",
      user_name: "alice",
      display_name: "Alice",
      created_at: "2026-01-01 00:00:00.000",
      updated_at: null,
    },
    {
      id: "user-bob",
      user_name: "bob",
      display_name: "Bob",
      created_at: "2026-01-01 00:00:00.000",
      updated_at: null,
    },
  ];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, dmChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  // 短い名前でも、リンクが行の幅いっぱいに広がる（右側の空き部分も押せる）。
  for (const link of [
    await screen.findByRole("link", { name: "general" }),
    await screen.findByRole("link", { name: "Alice, Bob" }),
  ]) {
    const row = link.parentElement!;
    const rowWidth = row.getBoundingClientRect().width;
    const linkWidth = link.getBoundingClientRect().width;
    expect(rowWidth).toBeGreaterThan(100);
    expect(Math.abs(linkWidth - rowWidth)).toBeLessThan(1);
  }
});

it("shows pinned channels and unpins them from the row menu", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "design" },
  ];
  fixture.pins = [pinRow("general", 0)];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  expect(await screen.findAllByRole("link", { name: "general" })).toHaveLength(2);
  const menus = screen.getAllByRole("button", { name: "general のチャンネル操作" });
  await userEvent.click(menus[0]!);
  await userEvent.click(await screen.findByRole("menuitem", { name: "ピン留めを解除" }));
  expect(fixture.unpinChannel).toHaveBeenCalledWith("general");
});

it("shows the thread tree under a pinned channel as well as in the channel list, but not under a pinned DM", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: dmChannel.id },
  ];
  fixture.pins = [pinRow("general", 0), pinRow(dmChannel.id, 1)];
  fixture.threads = [
    {
      id: "pinned-thread",
      channel_id: "general",
      content: "Pinned conversation",
      thread_archive_at: null,
      reply_count: 2,
    },
    {
      id: "archived-thread",
      channel_id: "general",
      content: "Archived conversation",
      thread_archive_at: "2020-01-01T00:00:00Z",
      reply_count: 4,
    },
    // 参加済みの DM に、アクティブなスレッドがあっても、ピン留めの下にツリーを出さない。
    {
      id: "dm-thread",
      channel_id: dmChannel.id,
      content: "DM conversation",
      thread_archive_at: null,
      reply_count: 1,
    },
  ];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, dmChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  const section = await screen.findByRole("region", { name: "ピン留め" });
  // ピン留めの行の下にも、通常の一覧と同じアクティブなスレッドを出す（アーカイブ済みは出さない）。
  const pinnedLink = await within(section).findByRole("link", { name: /Pinned conversation/ });
  expect(pinnedLink.getAttribute("href")).toBe("/channels/general?thread=pinned-thread");
  // 返信数は、ヘッダーの件数のバッジではなく、スレッドの行の中に出る。
  expect(within(pinnedLink).getByText("2")).toBeTruthy();
  expect(within(section).queryByRole("link", { name: /Archived conversation/ })).toBeNull();
  // 通常のチャンネル一覧の行のツリーも残る。
  expect(await screen.findAllByRole("link", { name: /Pinned conversation/ })).toHaveLength(2);
  // ピン留めした DM にはスレッドのツリーを出さない（DM のスレッドは、どこにも出ない）。
  expect(within(section).getAllByRole("link")).toHaveLength(3);
  expect(within(section).queryByRole("link", { name: /DM conversation/ })).toBeNull();
  expect(screen.queryByRole("link", { name: /DM conversation/ })).toBeNull();
  // スレッド一覧はサイドバー全体で共有し、ピン留めのために取り直さない。
  expect(fixture.get.mock.calls.filter(([path]) => path.startsWith("/api/threads"))).toHaveLength(
    1,
  );
});

it("shows NSFW and in-call indicators on pinned rows", async () => {
  fixture.memberships = [{ user_id: "me", channel_id: "general" }];
  fixture.pins = [pinRow("general", 0)];
  fixture.activeCalls = new Map([["general", 2]]);
  render(
    <LocaleProvider>
      <ChannelSidebar
        channels={[{ ...generalChannel, is_nsfw: true }]}
        loading={false}
        error={false}
      />
    </LocaleProvider>,
  );
  const section = await screen.findByRole("region", { name: "ピン留め" });
  expect(within(section).getByText("NSFW")).toBeTruthy();
  expect(within(section).getByRole("img", { name: "通話中" })).toBeTruthy();
});

it("keeps focus in the regular list when the last pin disappears", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "design" },
  ];
  fixture.pins = [pinRow("general", 0)];
  const view = () => (
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
    </LocaleProvider>
  );
  const { rerender } = render(view());
  const menus = await screen.findAllByRole("button", { name: "general のチャンネル操作" });
  await userEvent.click(menus[0]!);
  await userEvent.click(await screen.findByRole("menuitem", { name: "ピン留めを解除" }));
  expect(fixture.unpinChannel).toHaveBeenCalledWith("general");

  // 解除後にピン留めの節が消えても、同じチャンネルの通常一覧へフォーカスを移す。
  fixture.pins = [];
  rerender(view());
  await waitFor(() => {
    const focused = document.activeElement;
    expect(focused?.tagName).toBe("A");
    expect(focused?.getAttribute("href")).toBe("/channels/general");
  });
});

it("keeps focus in the regular list when a pin is removed while other pins remain", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "design" },
  ];
  fixture.pins = [pinRow("general", 0), pinRow("design", 1)];
  const view = () => (
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
    </LocaleProvider>
  );
  const { rerender } = render(view());
  const menus = await screen.findAllByRole("button", { name: "general のチャンネル操作" });
  await userEvent.click(menus[0]!);
  await userEvent.click(await screen.findByRole("menuitem", { name: "ピン留めを解除" }));
  expect(fixture.unpinChannel).toHaveBeenCalledWith("general");

  // ほかのピンが残って節が消えないときも、解除した行の近くへフォーカスを移す。
  fixture.pins = [pinRow("design", 0)];
  rerender(view());
  await waitFor(() => {
    const focused = document.activeElement;
    expect(focused?.tagName).toBe("A");
    expect(focused?.getAttribute("href")).toBe("/channels/general");
  });
});

it("keeps focus when the sidebar rerenders before the focus frame runs", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "design" },
  ];
  fixture.pins = [pinRow("general", 0), pinRow("design", 1)];
  const view = () => (
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
    </LocaleProvider>
  );
  const { rerender } = render(view());
  const menus = await screen.findAllByRole("button", { name: "general のチャンネル操作" });
  await userEvent.click(menus[0]!);
  await userEvent.click(await screen.findByRole("menuitem", { name: "ピン留めを解除" }));
  expect(fixture.unpinChannel).toHaveBeenCalledWith("general");

  // rAF が実行される前に依存値が続けて変わっても、予約した復旧を取り消さない。
  fixture.pins = [pinRow("design", 0)];
  rerender(view());
  fixture.pins = [pinRow("design", 0)];
  rerender(view());

  await waitFor(() => {
    const focused = document.activeElement;
    expect(focused?.tagName).toBe("A");
    expect(focused?.getAttribute("href")).toBe("/channels/general");
  });
});

it.each([
  { remainingPins: "none", pins: [] },
  { remainingPins: "other", pins: [pinRow("design", 0)] },
])(
  "keeps focus on the channel section heading when the regular list is collapsed (remaining pins: $remainingPins)",
  async ({ pins: remainingPins }) => {
    fixture.memberships = [
      { user_id: "me", channel_id: "general" },
      { user_id: "me", channel_id: "design" },
    ];
    fixture.pins = [pinRow("general", 0), pinRow("design", 1)];
    const view = () => (
      <LocaleProvider>
        <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
      </LocaleProvider>
    );
    const { rerender } = render(view());
    const heading = await screen.findByRole("button", { name: "チャンネル" });
    await userEvent.click(heading);
    expect(heading.getAttribute("aria-expanded")).toBe("false");

    const menus = screen.getAllByRole("button", { name: "general のチャンネル操作" });
    await userEvent.click(menus[0]!);
    await userEvent.click(await screen.findByRole("menuitem", { name: "ピン留めを解除" }));
    expect(fixture.unpinChannel).toHaveBeenCalledWith("general");

    // 折りたたまれた一覧のリンクはフォーカスを受け取れないため、節の見出しへ移す。
    fixture.pins = remainingPins;
    rerender(view());
    await waitFor(() => {
      expect(document.activeElement?.tagName).toBe("BUTTON");
      expect(document.activeElement).toBe(heading);
    });
  },
);

it("keeps focus on the pinned row menu while a failed unpin is pending", async () => {
  const actual = await vi.importActual<typeof import("../../hooks/usePinnedChannels")>(
    "../../hooks/usePinnedChannels",
  );
  // 実際の Provider とフックを使い、書き込み中の状態も含めて確かめる。
  vi.mocked(usePinnedChannels).mockImplementation(actual.usePinnedChannels);
  fixture.memberships = [
    { user_id: "me", channel_id: "general", position: 0 },
    { user_id: "me", channel_id: "design", position: 1 },
  ];
  fixture.get.mockImplementation(async (path: string) =>
    path.includes("/api/channel-pins")
      ? { pins: [pinRow("general", 0), pinRow("design", 1)], version: 1 }
      : { items: [], next_cursor: null },
  );
  // 解除は 200ms 待ってから失敗する。待っている間にメニューが閉じても、
  // 行のメニューボタンへフォーカスが戻ることを確かめる。
  fixture.del.mockImplementation(
    () =>
      new Promise((_resolve, reject) => {
        setTimeout(() => reject(new ApiError(500, "server error")), 200);
      }),
  );

  render(
    <LocaleProvider>
      <actual.PinnedChannelsProvider>
        <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
      </actual.PinnedChannelsProvider>
    </LocaleProvider>,
  );
  const section = await screen.findByRole("region", { name: "ピン留め" });
  const trigger = within(section).getByRole("button", { name: "general のチャンネル操作" });
  trigger.focus();
  await userEvent.keyboard("{Enter}");
  const unpin = await screen.findByRole("menuitem", { name: "ピン留めを解除" });
  unpin.focus();
  await userEvent.keyboard("{Enter}");

  await waitFor(() => expect(fixture.del).toHaveBeenCalled());
  // 待っている間も、メニューを閉じた時点でフォーカスは行のメニューへ戻る。
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(document.activeElement).not.toBe(document.body);

  // 解除に失敗して行が残ったあとも、フォーカスは同じメニューボタンにある。
  await screen.findByRole("alert");
  expect(document.activeElement).toBe(trigger);
});

it("keeps focus on the row menu while pinning from the regular list is pending", async () => {
  const actual = await vi.importActual<typeof import("../../hooks/usePinnedChannels")>(
    "../../hooks/usePinnedChannels",
  );
  vi.mocked(usePinnedChannels).mockImplementation(actual.usePinnedChannels);
  fixture.memberships = [
    { user_id: "me", channel_id: "general", position: 0 },
    { user_id: "me", channel_id: "design", position: 1 },
  ];
  const existing = [pinRow("general", 0)];
  const added = pinRow("design", 1);
  fixture.get.mockImplementation(async (path: string) =>
    path.includes("/api/channel-pins")
      ? { pins: existing, version: 1 }
      : { items: [], next_cursor: null },
  );
  // ピン留めは 200ms 待ってから成功する。
  fixture.post.mockImplementation(
    () =>
      new Promise((resolve) => {
        setTimeout(
          () =>
            resolve({
              data: { pin: added, version: 2, pins: [...existing, added] },
              txid: "2",
            }),
          200,
        );
      }),
  );

  render(
    <LocaleProvider>
      <actual.PinnedChannelsProvider>
        <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
      </actual.PinnedChannelsProvider>
    </LocaleProvider>,
  );
  const trigger = await screen.findByRole("button", { name: "design のチャンネル操作" });
  trigger.focus();
  await userEvent.keyboard("{Enter}");
  const pin = await screen.findByRole("menuitem", { name: "ピン留め" });
  pin.focus();
  await userEvent.keyboard("{Enter}");

  await waitFor(() => expect(fixture.post).toHaveBeenCalledTimes(1));
  // 書き込みが終わる前も、メニューを閉じた時点でフォーカスは行のメニューへ戻る。
  await waitFor(() => expect(document.activeElement).toBe(trigger));
  expect(document.activeElement).not.toBe(document.body);

  // 完了してピンの節に行が増えても、同じメニューボタンにフォーカスが残る。
  await waitFor(() =>
    expect(screen.getAllByRole("button", { name: "design のチャンネル操作" })).toHaveLength(2),
  );
  expect(document.activeElement).toBe(trigger);
});

it("pins a DM from the DM row menu", async () => {
  fixture.memberships = [{ user_id: "me", channel_id: dmChannel.id }];
  fixture.users = [
    {
      id: "user-alice",
      user_name: "alice",
      display_name: "Alice",
      created_at: "2026-01-01 00:00:00.000",
      updated_at: null,
    },
    {
      id: "user-bob",
      user_name: "bob",
      display_name: "Bob",
      created_at: "2026-01-01 00:00:00.000",
      updated_at: null,
    },
  ];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[dmChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  const menus = await screen.findAllByRole("button", { name: /の DM 操作/ });
  await userEvent.click(menus.at(-1)!);
  await userEvent.click(await screen.findByRole("menuitem", { name: "ピン留め" }));
  expect(fixture.pinChannel).toHaveBeenCalledWith("dm-sidebar");
});

it("reorders pinned channels with Alt+ArrowDown and drag and drop", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "design" },
  ];
  fixture.pins = [pinRow("general", 0), pinRow("design", 1)];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  const [first] = await screen.findAllByRole("link", { name: "general" });
  fireEvent.keyDown(first!, { key: "ArrowDown", altKey: true });
  expect(fixture.reorderPins).toHaveBeenCalledWith(["design", "general"], "general");

  fixture.reorderPins.mockClear();
  const firstRow = first!.closest('[draggable="true"]')!;
  const designs = await screen.findAllByRole("link", { name: "design" });
  expect(designs).toHaveLength(2);
  // 先頭のリンクがピン留めの行（ドラッグできる）、後ろが通常のチャンネル一覧。
  const secondRow = designs[0]!.closest('[draggable="true"]')!;
  const dataTransfer = new DataTransfer();
  const secondRect = secondRow.getBoundingClientRect();
  fireEvent.dragStart(firstRow, { dataTransfer });
  // 行の上半分では前に、下半分では後ろに挿入する線を出す。
  fireEvent.dragOver(secondRow, {
    dataTransfer,
    clientY: secondRect.top + 1,
  });
  // 線は、行とスレッドの行を囲むブロック（行の親）に出る。
  const secondBlock = secondRow.parentElement!;
  expect(secondBlock.querySelector('[data-drop-indicator="before"]')).not.toBeNull();
  fireEvent.dragOver(secondRow, {
    dataTransfer,
    clientY: secondRect.bottom - 1,
  });
  expect(secondBlock.querySelector('[data-drop-indicator="after"]')).not.toBeNull();
  fireEvent.drop(secondRow, { dataTransfer, clientY: secondRect.bottom - 1 });
  expect(fixture.reorderPins).toHaveBeenCalledWith(["design", "general"], "general");
});

it("treats a pinned channel and its thread rows as one drop target, with the line below the threads", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "design" },
  ];
  fixture.pins = [pinRow("design", 0), pinRow("general", 1)];
  fixture.threads = [
    {
      id: "t1",
      channel_id: "general",
      content: "First conversation",
      thread_archive_at: null,
      reply_count: 1,
    },
    {
      id: "t2",
      channel_id: "general",
      content: "Second conversation",
      thread_archive_at: null,
      reply_count: 2,
    },
  ];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  const section = await screen.findByRole("region", { name: "ピン留め" });
  const designRow = within(section)
    .getByRole("link", { name: "design" })
    .closest('[draggable="true"]')!;
  const generalRow = within(section)
    .getByRole("link", { name: "general" })
    .closest('[draggable="true"]')!;
  const block = generalRow.parentElement!;
  const lastThread = await within(block).findByRole("link", { name: /Second conversation/ });
  const dataTransfer = new DataTransfer();
  fireEvent.dragStart(designRow, { dataTransfer });

  // スレッドの行の上にも置ける。下半分なら、「後ろ」の線が、スレッドの行の下（ブロックの下端）に出る。
  const threadRect = lastThread.getBoundingClientRect();
  const accepted = !fireEvent.dragOver(lastThread, {
    dataTransfer,
    clientY: threadRect.bottom - 1,
  });
  expect(accepted).toBe(true);
  const after = block.querySelector<HTMLElement>('[data-drop-indicator="after"]');
  expect(after).not.toBeNull();
  const blockRect = block.getBoundingClientRect();
  const lineRect = after!.getBoundingClientRect();
  expect(Math.abs(lineRect.bottom - blockRect.bottom)).toBeLessThan(3);
  expect(lineRect.top).toBeGreaterThanOrEqual(threadRect.bottom - 3);
  // チャンネル行の中の線ではない。
  expect(generalRow.querySelector("[data-drop-indicator]")).toBeNull();

  // 上半分なら、「前」の線が、ブロックの上端に出る。
  fireEvent.dragOver(generalRow, { dataTransfer, clientY: blockRect.top + 1 });
  const before = block.querySelector<HTMLElement>('[data-drop-indicator="before"]');
  expect(before).not.toBeNull();
  expect(Math.abs(before!.getBoundingClientRect().top - blockRect.top)).toBeLessThan(3);

  // スレッドの行の下半分へ落とすと、そのチャンネルの後ろへ移る。
  fireEvent.drop(lastThread, { dataTransfer, clientY: threadRect.bottom - 1 });
  expect(fixture.reorderPins).toHaveBeenCalledWith(["general", "design"], "design");
});

it("ignores drags that do not come from a pinned row, such as a thread link", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "design" },
  ];
  fixture.pins = [pinRow("design", 0), pinRow("general", 1)];
  fixture.threads = [
    {
      id: "t1",
      channel_id: "general",
      content: "First conversation",
      thread_archive_at: null,
      reply_count: 1,
    },
  ];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, designChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  const section = await screen.findByRole("region", { name: "ピン留め" });
  const thread = await within(section).findByRole("link", { name: /First conversation/ });
  // ピン留めの行のドラッグを始めていない（スレッドのリンクなどをドラッグしている）。
  const dataTransfer = new DataTransfer();
  const rect = thread.getBoundingClientRect();
  const notCanceled = fireEvent.dragOver(thread, { dataTransfer, clientY: rect.bottom - 1 });
  expect(notCanceled).toBe(true);
  expect(section.querySelector("[data-drop-indicator]")).toBeNull();
  fireEvent.drop(thread, { dataTransfer, clientY: rect.bottom - 1 });
  expect(fixture.reorderPins).not.toHaveBeenCalled();
});

it("keeps an always mounted live region that announces pin operations", async () => {
  // 実際の Provider とフックを使い、読み上げ領域が操作後も更新されることを確認する。
  const actual = await vi.importActual<typeof import("../../hooks/usePinnedChannels")>(
    "../../hooks/usePinnedChannels",
  );
  fixture.memberships = [];
  fixture.get.mockImplementation(async (path: string) =>
    path.includes("/api/channel-pins")
      ? { pins: [], version: 0 }
      : { items: [], next_cursor: null },
  );
  fixture.post.mockResolvedValue({
    data: {
      pin: {
        user_id: "me",
        channel_id: "general",
        position: 0,
        archived: false,
        created_at: "2026-01-01 00:00:00.000",
        updated_at: "2026-01-01 00:00:00.000",
      },
      version: 1,
      pins: [
        {
          user_id: "me",
          channel_id: "general",
          position: 0,
          archived: false,
          created_at: "2026-01-01 00:00:00.000",
          updated_at: "2026-01-01 00:00:00.000",
        },
      ],
    },
    txid: "1",
  });

  function Harness() {
    const pins = actual.usePinnedChannels();
    return <button onClick={() => void pins.pinChannel("general")}>pin-harness</button>;
  }

  render(
    <LocaleProvider>
      <actual.PinnedChannelsProvider>
        <Harness />
      </actual.PinnedChannelsProvider>
    </LocaleProvider>,
  );
  const status = await screen.findByRole("status");
  expect(status.textContent).toBe("");

  await userEvent.click(screen.getByRole("button", { name: "pin-harness" }));
  await waitFor(() => expect(status.textContent).toBe("ピン留めしました。"));
});

it("clears a failed initial load when a write adopts the server's pin state", async () => {
  // 初回の GET が失敗しても、書き込みの応答でサーバーの状態を採用できたら、
  // 古い取得エラーの表示は残さない。
  const actual = await vi.importActual<typeof import("../../hooks/usePinnedChannels")>(
    "../../hooks/usePinnedChannels",
  );
  fixture.memberships = [{ user_id: "me", channel_id: "general", position: 0 }];
  fixture.get.mockImplementation(async (path: string) => {
    if (path.includes("/api/channel-pins")) throw new Error("server error");
    return { items: [], next_cursor: null };
  });
  const pinned = {
    user_id: "me",
    channel_id: "general",
    position: 0,
    archived: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: "2026-01-01 00:00:00.000",
  };
  fixture.post.mockResolvedValue({ data: { pin: pinned, version: 1, pins: [pinned] }, txid: "1" });

  function Harness() {
    const pins = actual.usePinnedChannels();
    return (
      <>
        <p data-testid="pins-error">{pins.error ?? "none"}</p>
        <button onClick={() => void pins.pinChannel("general")}>pin-harness</button>
      </>
    );
  }

  render(
    <LocaleProvider>
      <actual.PinnedChannelsProvider>
        <Harness />
      </actual.PinnedChannelsProvider>
    </LocaleProvider>,
  );
  await waitFor(() => expect(screen.getByTestId("pins-error").textContent).not.toBe("none"));
  await userEvent.click(screen.getByRole("button", { name: "pin-harness" }));
  await waitFor(() => expect(screen.getByTestId("pins-error").textContent).toBe("none"));
  expect(fixture.post).toHaveBeenCalledWith("/api/channel-pins", {
    channel_id: "general",
    expected_version: 0,
  });
});

it.each([
  { archived: true, announcement: "チャンネルがアーカイブされたため、移動しませんでした。" },
  { archived: false, announcement: "別の端末でピン留めが解除されたため、移動しませんでした。" },
])(
  "announces why a reorder was not applied (archived: $archived)",
  async ({ archived, announcement }) => {
    // 競合の本文でピンが外れている理由（アーカイブか解除か）を読み上げで区別する。
    const actual = await vi.importActual<typeof import("../../hooks/usePinnedChannels")>(
      "../../hooks/usePinnedChannels",
    );
    fixture.memberships = [];
    fixture.get.mockImplementation(async (path: string) =>
      path.includes("/api/channel-pins")
        ? { pins: [pinRow("general", 0), pinRow("design", 1)], version: 1 }
        : { items: [], next_cursor: null },
    );
    fixture.patch.mockRejectedValue(
      new ApiError(409, "conflict", {
        error: "conflict",
        code: "CHANNEL_PINS_CONFLICT",
        version: 2,
        pins: [
          { ...pinRow("design", 0), archived: false },
          ...(archived ? [{ ...pinRow("general", 1), archived: true }] : []),
        ],
      }),
    );

    function Harness() {
      const pins = actual.usePinnedChannels();
      return (
        <button onClick={() => void pins.reorderPins(["design", "general"], "general")}>
          reorder-harness
        </button>
      );
    }

    render(
      <LocaleProvider>
        <actual.PinnedChannelsProvider>
          <Harness />
        </actual.PinnedChannelsProvider>
      </LocaleProvider>,
    );
    await userEvent.click(await screen.findByRole("button", { name: "reorder-harness" }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe(announcement));
  },
);

it("hides archived pins from the pinned section and keeps the other pins", async () => {
  fixture.memberships = [
    { user_id: "me", channel_id: "general" },
    { user_id: "me", channel_id: "old-announcements" },
  ];
  fixture.pins = [pinRow("old-announcements", 0), pinRow("general", 1)];
  render(
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel, archivedChannel]} loading={false} error={false} />
    </LocaleProvider>,
  );
  const section = await screen.findByRole("region", { name: "ピン留め" });
  expect(within(section).getByRole("link", { name: "general" })).toBeTruthy();
  expect(within(section).queryByRole("link", { name: "old-announcements" })).toBeNull();
});

it("shows a dismissible alert when pinning fails", async () => {
  fixture.memberships = [{ user_id: "me", channel_id: "general" }];
  fixture.pins = [pinRow("general", 0)];
  fixture.pinError = "ピン留めに失敗しました。";
  const view = () => (
    <LocaleProvider>
      <ChannelSidebar channels={[generalChannel]} loading={false} error={false} />
    </LocaleProvider>
  );
  const { rerender } = render(view());
  expect((await screen.findByRole("alert")).textContent).toContain("ピン留めに失敗しました。");
  await userEvent.click(screen.getByRole("button", { name: "閉じる" }));
  expect(fixture.dismissError).toHaveBeenCalled();

  fixture.pinError = null;
  rerender(view());
  expect(screen.queryByRole("alert")).toBeNull();
});
