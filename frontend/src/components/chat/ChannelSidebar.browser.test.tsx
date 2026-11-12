import "../../styles.css";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { ChannelSidebar } from "./ChannelSidebar";
import type { ChannelRow } from "../../lib/electric/types";
import type { ThreadSummary } from "../../hooks/useThreadIndex";

const fixture = vi.hoisted(() => ({
  memberships: [{ user_id: "other", channel_id: "general" }],
  threads: [] as ThreadSummary[],
  navigate: vi.fn(),
  config: {},
  get: vi.fn(),
  selectedId: null as string | null,
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
vi.mock("../../hooks/useRealtime", () => ({ useUsers: () => ({ data: [], isLoading: false }) }));
vi.mock("../../hooks/useUnreadChannels", () => ({ useUnreadChannels: () => new Set() }));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ channelUsers: {}, config: fixture.config }),
}));
vi.mock("../../lib/api/client", () => ({
  createConfiguredApiClient: () => ({
    get: fixture.get,
    post: vi.fn(),
  }),
}));
vi.mock("../../lib/call/call-context", () => ({ useCall: () => ({ activeCalls: new Map() }) }));
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
vi.mock("./DmSection", () => ({ DmSection: () => null }));

beforeEach(() => {
  fixture.memberships = [{ user_id: "other", channel_id: "general" }];
  fixture.threads = [];
  fixture.selectedId = null;
  fixture.navigate.mockClear();
  fixture.config = {};
  fixture.get.mockReset().mockImplementation(async (path: string) => ({
    items: fixture.threads.filter(
      (thread) => !!thread.thread_archive_at === path.includes("archived=true"),
    ),
    next_cursor: null,
  }));
});

it("waits for the current user's membership even when other users are cached", () => {
  const channel = { id: "general", channel_name: "general", is_dm: false } as ChannelRow;
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

it("temporarily shows the selected archive without any history or search controls", async () => {
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
        channels={[{ id: "general", channel_name: "general", is_dm: false } as ChannelRow]}
        loading={false}
        error={false}
      />
    </LocaleProvider>
  );
  const { rerender } = render(view());
  await screen.findByRole("link", { name: /Active conversation/ });
  const selected = await screen.findByRole("link", { name: /Old conversation/ });
  expect(selected.getAttribute("aria-current")).toBe("page");
  expect(screen.queryByRole("button", { name: /アーカイブ済みスレッド/ })).toBeNull();
  expect(screen.queryByRole("searchbox")).toBeNull();
  expect(fixture.get.mock.calls.every(([path]) => !path.includes("archived=true"))).toBe(true);
  fixture.selectedId = null;
  rerender(view());
  expect(screen.queryByRole("link", { name: /Old conversation/ })).toBeNull();
});
