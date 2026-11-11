import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { userEvent } from "vite-plus/test/browser/context";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { ChannelSidebar } from "./ChannelSidebar";
import type { ChannelRow } from "../../lib/electric/types";
import type { ThreadSummary } from "../../hooks/useThreadIndex";

const fixture = vi.hoisted(() => ({
  memberships: [{ user_id: "other", channel_id: "general" }],
  threads: [] as ThreadSummary[],
  navigate: vi.fn(),
  config: {},
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
    get: async (path: string) => ({
      items: fixture.threads.filter(
        (thread) => !!thread.thread_archive_at === path.includes("archived=true"),
      ),
      next_cursor: null,
    }),
    post: vi.fn(),
  }),
}));
vi.mock("../../lib/call/call-context", () => ({ useCall: () => ({ activeCalls: new Map() }) }));
vi.mock("../layout/shell-context", () => ({
  useShell: () => ({ selectedChannelId: null, showChat: vi.fn() }),
}));
vi.mock("../SyncStatus", () => ({ SyncStatus: () => null }));
vi.mock("./ChannelBrowserDialog", () => ({ ChannelBrowserDialog: () => null }));
vi.mock("./CreateChannelDialog", () => ({ CreateChannelDialog: () => null }));
vi.mock("./CreateDmDialog", () => ({ CreateDmDialog: () => null }));
vi.mock("./DmSection", () => ({ DmSection: () => null }));

beforeEach(() => {
  fixture.memberships = [{ user_id: "other", channel_id: "general" }];
  fixture.threads = [];
  fixture.navigate.mockClear();
});

it("opens archived threads with the shared section and keyboard controls", async () => {
  fixture.memberships = [{ user_id: "me", channel_id: "general" }];
  fixture.threads = [
    {
      id: "child-thread",
      channel_id: "general",
      content: "Nested history",
      thread_archive_at: "2020-02-01T00:00:00Z",
      reply_count: 2,
    },
  ];
  render(
    <LocaleProvider>
      <ChannelSidebar
        channels={[{ id: "general", channel_name: "general", is_dm: false } as ChannelRow]}
        loading={false}
        error={false}
      />
    </LocaleProvider>,
  );
  const toggle = await screen.findByRole("button", { name: /アーカイブ済みスレッド/ });
  toggle.focus();
  await userEvent.keyboard("{Enter}");
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  await screen.findByRole("link", { name: /Nested history/ });
  await userEvent.tab();
  expect(document.activeElement).toBe(screen.getByRole("link", { name: /Nested history/ }));
  await userEvent.keyboard("{Enter}");
  await waitFor(() =>
    expect(fixture.navigate).toHaveBeenCalledWith({
      to: "/channels/$channelId",
      params: { channelId: "general" },
      search: { thread: "child-thread" },
    }),
  );
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
