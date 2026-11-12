import "../../styles.css";
import { act } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createRootRoute,
  createRoute,
  createRouter,
  createMemoryHistory,
  RouterProvider,
  Outlet,
  useRouterState,
  useNavigate,
} from "@tanstack/react-router";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { ChannelThreadItems } from "./ChannelThreadItems";
import { ShellContext } from "../layout/shell-context";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { useThreadIndex, type ThreadPage } from "../../hooks/useThreadIndex";
import type { ChannelRow } from "../../lib/electric/types";

const fixture = vi.hoisted(() => ({
  config: {},
  loadMore: () => {},
  hasMore: false,
  get: vi.fn(),
  post: vi.fn(),
  roots: [
    {
      id: "old",
      channel_id: "general",
      content: "Old archived conversation",
      thread_archive_at: "2020-01-01T00:00:00Z",
      reply_count: 2,
    },
    {
      id: "other",
      channel_id: "general",
      content: "Other archived conversation",
      thread_archive_at: "2020-01-01T00:00:00Z",
      reply_count: 1,
    },
    {
      id: "active",
      channel_id: "general",
      content: "Active conversation",
      thread_archive_at: null,
      reply_count: 1,
    },
  ],
}));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: fixture.config }),
}));
vi.mock("../../lib/api/client", () => ({
  createConfiguredApiClient: () => ({ get: fixture.get, post: fixture.post }),
}));
vi.mock("../../hooks/useThread", () => ({
  useThreadRoot: (_channel: string | null, id: string | null) => ({
    data: fixture.roots
      .filter((root) => root.id === id)
      .map((root) => ({ ...root, display_data: { reply_count: root.reply_count } })),
  }),
}));

function Channel() {
  const id = useRouterState({
    select: (state) => (state.location.search as { thread?: string }).thread ?? null,
  });
  const navigate = useNavigate();
  const openThread = (thread: string) => {
    void navigate({
      to: "/channels/$channelId",
      params: { channelId: "general" },
      search: { thread },
    });
  };
  const closeThread = () => {
    void navigate({ to: "/channels/$channelId", params: { channelId: "general" }, search: {} });
  };
  const index = useThreadIndex("me");
  fixture.loadMore = index.loadMore;
  fixture.hasMore = index.hasMore;
  return (
    <ShellContext.Provider
      value={{
        pane: "chat",
        showChat: () => {},
        showSidebar: () => {},
        membersOpen: false,
        setMembersOpen: () => {},
        toggleMembers: () => {},
        selectedChannelId: "general",
        threadRootId: id,
        openThread,
        closeThread,
      }}
    >
      <aside aria-label="Thread tree">
        <ChannelThreadItems
          channel={{ id: "general", channel_name: "general" } as ChannelRow}
          threads={index.threads}
        />
      </aside>
      <button onClick={() => openThread("old")}>Reply to parent</button>
      <button onClick={closeThread}>Close conversation</button>
      {id && <p>Viewing:{id}</p>}
    </ShellContext.Provider>
  );
}

function setup(url: string) {
  const root = createRootRoute({
    component: () => (
      <LocaleProvider>
        <Outlet />
      </LocaleProvider>
    ),
  });
  const channel = createRoute({
    getParentRoute: () => root,
    path: "/channels/$channelId",
    validateSearch: (search: Record<string, unknown>) => ({
      thread: typeof search.thread === "string" ? search.thread : undefined,
    }),
    component: Channel,
  });
  const router = createRouter({
    routeTree: root.addChildren([channel]),
    history: createMemoryHistory({ initialEntries: [url] }),
  });
  const view = render(<RouterProvider router={router} />);
  return { router, ...view };
}

beforeEach(() => {
  fixture.config = {};
  fixture.get.mockReset().mockImplementation(async (path: string) => ({
    items: fixture.roots.filter(
      (root) => !!root.thread_archive_at === path.includes("archived=true"),
    ),
    next_cursor: null,
  }));
  fixture.post.mockClear();
});

it("opens from a parent, restores the URL, and removes the temporary entry on navigation and close", async () => {
  const first = setup("/channels/general");
  const tree = within(await screen.findByRole("complementary", { name: "Thread tree" }));
  await tree.findByRole("link", { name: /Active conversation/ });
  expect(tree.queryByRole("link", { name: /archived conversation/ })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Reply to parent" }));
  expect(
    (await tree.findByRole("link", { name: /Old archived conversation/ })).getAttribute(
      "aria-current",
    ),
  ).toBe("page");
  const url = first.router.state.location.href;
  first.unmount();
  const restored = setup(url);
  const restoredTree = within(await screen.findByRole("complementary", { name: "Thread tree" }));
  await restoredTree.findByRole("link", { name: /Old archived conversation/ });
  expect(screen.getByText("Viewing:old")).toBeTruthy();
  await restored.router.navigate({
    to: "/channels/$channelId",
    params: { channelId: "general" },
    search: { thread: "other" },
  });
  await restoredTree.findByRole("link", { name: /Other archived conversation/ });
  expect(restoredTree.queryByRole("link", { name: /Old archived conversation/ })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Close conversation" }));
  await waitFor(() =>
    expect(restoredTree.queryByRole("link", { name: /Other archived conversation/ })).toBeNull(),
  );
  expect(fixture.get).toHaveBeenCalledTimes(2);
  expect(fixture.get.mock.calls.every(([path]) => path === "/api/threads")).toBe(true);
  expect(fixture.post).not.toHaveBeenCalled();
});

it("has no archive history or search entry and never requests an archive list", async () => {
  setup("/channels/general");
  const tree = within(await screen.findByRole("complementary", { name: "Thread tree" }));
  await tree.findByRole("link", { name: /Active conversation/ });
  expect(tree.queryByRole("button", { name: /アーカイブ済みスレッド/ })).toBeNull();
  expect(tree.queryByRole("searchbox")).toBeNull();
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(fixture.get.mock.calls.every(([path]) => !path.includes("archived=true"))).toBe(true);
});

it("keeps a URL-selected archive visible when the list fails", async () => {
  fixture.get.mockRejectedValue(new Error("offline"));
  setup("/channels/general?thread=old");
  await screen.findByRole("link", { name: /Old archived conversation/ });
  expect(screen.getByText("Viewing:old")).toBeTruthy();
  expect(fixture.post).not.toHaveBeenCalled();
});

it("removes a viewed archive on navigation even when the active list still contains its old state", async () => {
  fixture.get.mockResolvedValue({
    items: [{ ...fixture.roots[0], thread_archive_at: null }],
    next_cursor: null,
  });
  const { router } = setup("/channels/general?thread=old");
  const tree = within(await screen.findByRole("complementary", { name: "Thread tree" }));
  await tree.findByRole("link", { name: /Old archived conversation/ });
  await router.navigate({
    to: "/channels/$channelId",
    params: { channelId: "general" },
    search: { thread: "other" },
  });
  await tree.findByRole("link", { name: /Other archived conversation/ });
  expect(tree.queryByRole("link", { name: /Old archived conversation/ })).toBeNull();
  expect(fixture.post).not.toHaveBeenCalled();
});

it("keeps observed archives hidden after appending a page", async () => {
  fixture.get
    .mockReset()
    .mockResolvedValueOnce({
      items: [{ ...fixture.roots[0], thread_archive_at: null }],
      next_cursor: "next",
    })
    .mockResolvedValue({ items: [fixture.roots[2]], next_cursor: null });
  const { router } = setup("/channels/general?thread=old");
  const tree = within(await screen.findByRole("complementary", { name: "Thread tree" }));
  await waitFor(() => expect(fixture.hasMore).toBe(true));
  await tree.findByRole("link", { name: /Old archived conversation/ });
  await router.navigate({
    to: "/channels/$channelId",
    params: { channelId: "general" },
    search: { thread: "other" },
  });
  await tree.findByRole("link", { name: /Other archived conversation/ });
  expect(tree.queryByRole("link", { name: /Old archived conversation/ })).toBeNull();
  await act(async () => fixture.loadMore());
  await tree.findByRole("link", { name: /Active conversation/ });
  expect(fixture.get.mock.calls.at(-1)?.[0]).toContain("before=next");
  expect(tree.queryByRole("link", { name: /Old archived conversation/ })).toBeNull();
  expect(fixture.post).not.toHaveBeenCalled();
});

it("accepts a freshly fetched active state after preserving an archive correction", async () => {
  fixture.get
    .mockReset()
    .mockResolvedValueOnce({
      items: [{ ...fixture.roots[0], thread_archive_at: null }],
      next_cursor: "next",
    })
    .mockResolvedValue({
      items: [{ ...fixture.roots[0], thread_archive_at: null }],
      next_cursor: null,
    });
  const { router } = setup("/channels/general?thread=old");
  const tree = within(await screen.findByRole("complementary", { name: "Thread tree" }));
  await waitFor(() => expect(fixture.hasMore).toBe(true));
  await router.navigate({
    to: "/channels/$channelId",
    params: { channelId: "general" },
    search: { thread: "other" },
  });
  await tree.findByRole("link", { name: /Other archived conversation/ });
  expect(tree.queryByRole("link", { name: /Old archived conversation/ })).toBeNull();
  await act(async () => window.dispatchEvent(new Event("focus")));
  await tree.findByRole("link", { name: /Old archived conversation/ });
  expect(fixture.post).not.toHaveBeenCalled();
});

it("keeps an observed archive hidden when an earlier refresh finishes late", async () => {
  const archiveAt = fixture.roots[0].thread_archive_at;
  const stale = { ...fixture.roots[0], thread_archive_at: null };
  fixture.roots[0].thread_archive_at = null;
  let resolveRefresh!: (value: ThreadPage) => void;
  fixture.get
    .mockReset()
    .mockResolvedValueOnce({ items: [stale], next_cursor: "next" })
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveRefresh = resolve;
        }),
    );
  try {
    const { router } = setup("/channels/general?thread=old");
    const tree = within(await screen.findByRole("complementary", { name: "Thread tree" }));
    await waitFor(() => expect(fixture.hasMore).toBe(true));
    await act(async () => window.dispatchEvent(new Event("focus")));
    await waitFor(() => expect(resolveRefresh).toBeDefined());
    fixture.roots[0].thread_archive_at = archiveAt;
    const navigate = (thread: string) =>
      router.navigate({
        to: "/channels/$channelId",
        params: { channelId: "general" },
        search: { thread },
      });
    await navigate("other");
    await navigate("old");
    await tree.findByRole("link", { name: /Old archived conversation/ });
    await navigate("other");
    await tree.findByRole("link", { name: /Other archived conversation/ });
    expect(tree.queryByRole("link", { name: /Old archived conversation/ })).toBeNull();
    await act(async () =>
      resolveRefresh({ items: [{ ...stale }, { ...fixture.roots[2] }], next_cursor: null }),
    );
    await tree.findByRole("link", { name: /Active conversation/ });
    expect(tree.queryByRole("link", { name: /Old archived conversation/ })).toBeNull();
    expect(fixture.get).toHaveBeenCalledTimes(2);
    expect(fixture.post).not.toHaveBeenCalled();
  } finally {
    fixture.roots[0].thread_archive_at = archiveAt;
  }
});
