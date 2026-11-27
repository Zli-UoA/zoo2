import "../../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createBrowserHistory,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { page } from "vite-plus/test/browser/context";

import { DEFAULT_CONFIG } from "../../../lib/config";
import { mockServer, startMockServer } from "../../../test/browser-setup";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ChannelScreen } from "../ChannelScreen";
import { ChannelLink } from "../messages/ContentLink";
import type { ChannelLinkTarget } from "../messages/messageLink";
import { ChannelNotesScreen } from "./ChannelNotesScreen";

const channelScreenState = vi.hoisted(() => ({
  channel: {
    id: "11111111-1111-4111-8111-111111111111",
    channel_name: "notes",
    is_dm: false,
    is_private: false,
    encrypted: false,
    posting_policy: "members",
    archive_at: null,
  },
  access: {
    members: [],
    canRead: true,
    isMembershipPending: false,
    isMember: true,
    isArchived: false,
    canManageChannel: false,
    isLoading: false,
    isSyncing: false,
  },
}));

vi.mock("../../../hooks/useRealtime", () => ({
  useActiveChannels: () => ({
    data: [channelScreenState.channel],
    isInitialSyncLoading: false,
    isLoading: false,
    isError: false,
  }),
  useUsers: () => ({ data: [] }),
  useAllUsers: () => ({ data: [] }),
  useChannelAccess: () => channelScreenState.access,
}));
vi.mock("../../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "alice", isLoading: false, isAdmin: false }),
}));
vi.mock("../../layout/shell-context", () => ({
  useShell: () => ({ showSidebar: vi.fn() }),
}));
vi.mock("../ChannelHeader", () => ({
  ChannelHeader: ({ channel }: { channel: { channel_name: string } }) => (
    <h1>{channel.channel_name}</h1>
  ),
}));
vi.mock("../messages/MessagePane", () => ({
  MessagePane: ({
    targetMessageId,
    isActive,
  }: {
    targetMessageId?: string;
    isActive?: boolean;
  }) => (
    <p>
      messages:{targetMessageId ?? "none"}:{String(isActive)}
    </p>
  ),
}));

vi.mock("../../../lib/electric/collections-context", async (importOriginal) => {
  const config = { ...DEFAULT_CONFIG, telemetryEnabled: false };
  return {
    ...(await importOriginal<typeof import("../../../lib/electric/collections-context")>()),
    useCollections: () => ({ config }),
  };
});

const CHANNEL = "11111111-1111-4111-8111-111111111111";

/** 実際の `ChannelLink` を、チャンネル画面のルートと同じ検索パラメーターで描く。 */
function renderChannelLinkRoute(
  initialEntry: string,
  links: Array<{ label: string; target: ChannelLinkTarget }>,
) {
  const history = createMemoryHistory({ initialEntries: [initialEntry] });
  const root = createRootRoute({ component: () => <Outlet /> });
  function ChannelPage() {
    const search = useRouterState({
      select: (state) => state.location.search as { message?: string; thread?: string },
    });
    return (
      <LocaleProvider>
        <ChannelScreen
          channelId={CHANNEL}
          targetMessageId={search.message}
          targetThreadId={search.thread}
        />
        {links.map((link) => (
          <ChannelLink key={link.label} target={link.target}>
            {link.label}
          </ChannelLink>
        ))}
      </LocaleProvider>
    );
  }
  const channelRoute = createRoute({
    getParentRoute: () => root,
    path: "/channels/$channelId",
    validateSearch: (search: Record<string, unknown>) => ({
      message: typeof search.message === "string" ? search.message : undefined,
      thread: typeof search.thread === "string" ? search.thread : undefined,
    }),
    component: ChannelPage,
  });
  const router = createRouter({
    routeTree: root.addChildren([channelRoute]),
    history,
  });
  const view = render(<RouterProvider router={router} />);
  return { router, view };
}

beforeEach(async () => {
  await startMockServer();
  await page.viewport(1024, 800);
});

afterEach(() => {
  vi.restoreAllMocks();
});

it.each(["back", "push"] as const)(
  "preserves or discards a note draft on browser %s navigation",
  async (action) => {
    mockServer.use(
      http.get("*/api/channels/:channelId/notes", () =>
        HttpResponse.json({ data: [], txid: null }),
      ),
    );
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const originalUrl = window.location.href;
    const originalState = window.history.state;
    const history =
      action === "back"
        ? createBrowserHistory()
        : createMemoryHistory({ initialEntries: ["/previous", "/notes"] });
    if (action === "back") {
      history.replace("/previous");
      history.flush();
      history.push("/notes");
      history.flush();
    }
    const unloadBlocked = () =>
      !window.dispatchEvent(new Event("beforeunload", { cancelable: true }));

    const root = createRootRoute({
      component: () => (
        <LocaleProvider>
          <Outlet />
        </LocaleProvider>
      ),
    });
    const previous = createRoute({
      getParentRoute: () => root,
      path: "/previous",
      component: () => <h1>Previous channel</h1>,
    });
    const next = createRoute({
      getParentRoute: () => root,
      path: "/next",
      component: () => <h1>Next channel</h1>,
    });
    const runNavigation = () => {
      if (action === "back") history.back();
      else void router.navigate({ to: "/next" as never });
    };
    const notes = createRoute({
      getParentRoute: () => root,
      path: "/notes",
      component: () => (
        <div style={{ display: "flex", height: 600, width: 1024 }}>
          <ChannelNotesScreen
            channelId={CHANNEL}
            currentUserId="alice"
            canRead
            canWrite
            isMember
            isArchived={false}
          />
          <button type="button" onClick={runNavigation}>
            Change channel
          </button>
        </div>
      ),
    });
    const router = createRouter({
      routeTree: root.addChildren([previous, notes, next]),
      history,
    });
    const view = render(<RouterProvider router={router} />);
    try {
      const create = await screen.findByRole("button", { name: "新規ノート" });
      await waitFor(() => expect(create).toBeEnabled());
      if (action === "back") expect(unloadBlocked()).toBe(false);

      await userEvent.click(create);
      await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Unsaved text");
      if (action === "back") expect(unloadBlocked()).toBe(true);

      await userEvent.click(screen.getByRole("button", { name: "Change channel" }));
      await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
      expect(router.state.location.pathname).toBe("/notes");
      expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Unsaved text");

      confirm.mockReturnValue(true);
      await userEvent.click(screen.getByRole("button", { name: "Change channel" }));
      expect(
        await screen.findByRole("heading", {
          name: action === "back" ? "Previous channel" : "Next channel",
        }),
      ).toBeVisible();
      expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull();
      if (action === "back") expect(unloadBlocked()).toBe(false);
    } finally {
      view.unmount();
      history.destroy();
      if (action === "back") {
        // 戻った位置から元の URL を追加し、テスト用の進む履歴を切り捨てる。
        window.history.pushState(originalState, "", originalUrl);
      }
    }
  },
);

it("switches to messages only after a same-channel message navigation is accepted", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => HttpResponse.json({ data: [], txid: null })),
  );
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  const history = createMemoryHistory({ initialEntries: ["/channel"] });
  const root = createRootRoute({ component: () => <Outlet /> });
  function ChannelPage() {
    const search = useRouterState({
      select: (state) => state.location.search as { message?: string },
    });
    const navigate = useNavigate();
    return (
      <LocaleProvider>
        <ChannelScreen channelId={CHANNEL} targetMessageId={search.message} />
        <button
          type="button"
          onClick={() => void navigate({ to: "/channel", search: { message: "m2" } } as never)}
        >
          Open message
        </button>
      </LocaleProvider>
    );
  }
  const channelRoute = createRoute({
    getParentRoute: () => root,
    path: "/channel",
    validateSearch: (search: Record<string, unknown>) => ({
      message: typeof search.message === "string" ? search.message : undefined,
    }),
    component: ChannelPage,
  });
  const router = createRouter({
    routeTree: root.addChildren([channelRoute]),
    history,
  });
  const view = render(<RouterProvider router={router} />);
  try {
    await userEvent.click(await screen.findByRole("tab", { name: "ノート" }));
    const create = await screen.findByRole("button", { name: "新規ノート" });
    await waitFor(() => expect(create).not.toBeDisabled());
    await userEvent.click(create);
    await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Unsaved text");

    await userEvent.click(screen.getByRole("button", { name: "Open message" }));
    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Unsaved text");
    expect(router.state.location.pathname).toBe("/channel");
    expect((router.state.location.search as { message?: string }).message).toBeUndefined();

    confirm.mockReturnValue(true);
    await userEvent.click(screen.getByRole("button", { name: "Open message" }));
    expect(await screen.findByText("messages:m2:true")).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull();
    expect((router.state.location.search as { message?: string }).message).toBe("m2");
  } finally {
    view.unmount();
  }
});

it("returns from notes on a normal click of the already-open message link", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => HttpResponse.json({ data: [], txid: null })),
  );
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  const { router, view } = renderChannelLinkRoute(`/channels/${CHANNEL}?message=m1`, [
    {
      label: "同じメッセージを開く",
      target: { channelId: CHANNEL, search: { message: "m1" } },
    },
  ]);
  try {
    await userEvent.click(await screen.findByRole("tab", { name: "ノート" }));
    const create = await screen.findByRole("button", { name: "新規ノート" });
    await waitFor(() => expect(create).not.toBeDisabled());
    await userEvent.click(create);
    await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Unsaved text");

    const link = screen.getByRole("link", { name: "同じメッセージを開く" });
    await userEvent.click(link);
    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Unsaved text");
    expect(screen.getByRole("tab", { name: "ノート" })).toHaveAttribute("aria-selected", "true");
    expect((router.state.location.search as { message?: string }).message).toBe("m1");

    confirm.mockReturnValue(true);
    await userEvent.click(link);
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull());
    expect(screen.getByRole("tab", { name: "メッセージ" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(confirm).toHaveBeenCalledTimes(2);
  } finally {
    view.unmount();
  }
});

it("does not return from notes before a different message link is accepted", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => HttpResponse.json({ data: [], txid: null })),
  );
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  const { router, view } = renderChannelLinkRoute(`/channels/${CHANNEL}?message=m1`, [
    {
      label: "別のメッセージを開く",
      target: { channelId: CHANNEL, search: { message: "m2" } },
    },
  ]);
  try {
    await userEvent.click(await screen.findByRole("tab", { name: "ノート" }));
    const create = await screen.findByRole("button", { name: "新規ノート" });
    await waitFor(() => expect(create).not.toBeDisabled());
    await userEvent.click(create);
    await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Unsaved text");

    const link = screen.getByRole("link", { name: "別のメッセージを開く" });
    await userEvent.click(link);
    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Unsaved text");
    expect(screen.getByRole("tab", { name: "ノート" })).toHaveAttribute("aria-selected", "true");
    expect((router.state.location.search as { message?: string }).message).toBe("m1");

    confirm.mockReturnValue(true);
    await userEvent.click(link);
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull());
    expect(screen.getByRole("tab", { name: "メッセージ" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect((router.state.location.search as { message?: string }).message).toBe("m2");
    expect(confirm).toHaveBeenCalledTimes(2);
  } finally {
    view.unmount();
  }
});

it("returns from notes on a normal click of the already-open thread link", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => HttpResponse.json({ data: [], txid: null })),
  );
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  const { router, view } = renderChannelLinkRoute(`/channels/${CHANNEL}?thread=t1`, [
    {
      label: "同じスレッドを開く",
      target: { channelId: CHANNEL, search: { thread: "t1" } },
    },
  ]);
  try {
    await userEvent.click(await screen.findByRole("tab", { name: "ノート" }));
    const create = await screen.findByRole("button", { name: "新規ノート" });
    await waitFor(() => expect(create).not.toBeDisabled());
    await userEvent.click(create);
    await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Unsaved text");

    const link = screen.getByRole("link", { name: "同じスレッドを開く" });
    await userEvent.click(link);
    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Unsaved text");
    expect(screen.getByRole("tab", { name: "ノート" })).toHaveAttribute("aria-selected", "true");
    expect((router.state.location.search as { thread?: string }).thread).toBe("t1");

    confirm.mockReturnValue(true);
    await userEvent.click(link);
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull());
    expect(screen.getByRole("tab", { name: "メッセージ" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(confirm).toHaveBeenCalledTimes(2);
  } finally {
    view.unmount();
  }
});

it("does not return from notes before a different thread-only link is accepted", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => HttpResponse.json({ data: [], txid: null })),
  );
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  const { router, view } = renderChannelLinkRoute(`/channels/${CHANNEL}?thread=t1`, [
    {
      label: "別のスレッドを開く",
      target: { channelId: CHANNEL, search: { thread: "t2" } },
    },
  ]);
  try {
    await userEvent.click(await screen.findByRole("tab", { name: "ノート" }));
    const create = await screen.findByRole("button", { name: "新規ノート" });
    await waitFor(() => expect(create).not.toBeDisabled());
    await userEvent.click(create);
    await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Unsaved text");

    const link = screen.getByRole("link", { name: "別のスレッドを開く" });
    await userEvent.click(link);
    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Unsaved text");
    expect(screen.getByRole("tab", { name: "ノート" })).toHaveAttribute("aria-selected", "true");
    expect((router.state.location.search as { thread?: string }).thread).toBe("t1");

    confirm.mockReturnValue(true);
    await userEvent.click(link);
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull());
    expect(screen.getByRole("tab", { name: "メッセージ" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect((router.state.location.search as { thread?: string }).thread).toBe("t2");
    expect((router.state.location.search as { message?: string }).message).toBeUndefined();
    expect(confirm).toHaveBeenCalledTimes(2);
  } finally {
    view.unmount();
  }
});

it("returns from notes when only the thread changes after acceptance", async () => {
  mockServer.use(
    http.get("*/api/channels/:channelId/notes", () => HttpResponse.json({ data: [], txid: null })),
  );
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  const { router, view } = renderChannelLinkRoute(`/channels/${CHANNEL}?message=m1&thread=t1`, [
    {
      label: "同じメッセージの別スレッドを開く",
      target: { channelId: CHANNEL, search: { message: "m1", thread: "t2" } },
    },
  ]);
  try {
    await userEvent.click(await screen.findByRole("tab", { name: "ノート" }));
    const create = await screen.findByRole("button", { name: "新規ノート" });
    await waitFor(() => expect(create).not.toBeDisabled());
    await userEvent.click(create);
    await userEvent.type(screen.getByRole("textbox", { name: "本文" }), "Unsaved text");

    const link = screen.getByRole("link", { name: "同じメッセージの別スレッドを開く" });
    await userEvent.click(link);
    await waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("textbox", { name: "本文" })).toHaveValue("Unsaved text");
    expect(screen.getByRole("tab", { name: "ノート" })).toHaveAttribute("aria-selected", "true");
    expect((router.state.location.search as { message?: string }).message).toBe("m1");
    expect((router.state.location.search as { thread?: string }).thread).toBe("t1");

    confirm.mockReturnValue(true);
    await userEvent.click(link);
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "本文" })).toBeNull());
    expect(screen.getByRole("tab", { name: "メッセージ" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect((router.state.location.search as { message?: string }).message).toBe("m1");
    expect((router.state.location.search as { thread?: string }).thread).toBe("t2");
    expect(confirm).toHaveBeenCalledTimes(2);
  } finally {
    view.unmount();
  }
});
