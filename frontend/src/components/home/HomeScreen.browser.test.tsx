import "../../styles.css";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { HomeScreen, type HomePost } from "./HomeScreen";
import { ApiError } from "../../lib/api/client";

const fixture = vi.hoisted(() => ({
  get: vi.fn(),
  request: vi.fn(),
  config: { apiBaseUrl: "https://home.test", devUserId: "me" },
  user: "me",
  live: () => {},
}));
vi.mock("../../lib/api/client", () => ({
  ApiError: class extends Error {
    status = 500;
  },
  createConfiguredApiClient: () => ({
    get: fixture.get,
    request: fixture.request,
  }),
}));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: fixture.config }),
  useOptionalCollections: () => ({ config: fixture.config }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: fixture.user }),
}));
vi.mock("../../hooks/useRealtime", () => ({ useUsers: () => ({ data: [] }) }));
vi.mock("../../hooks/useHomeLive", () => ({
  useHomeLive: (callback: () => void) => {
    fixture.live = callback;
  },
}));
// These controls are independent of timeline layout and data loading.
vi.mock("./HomeComposer", () => ({ HomeComposer: () => null }));
vi.mock("./HomeReactionPicker", () => ({ HomeReactionPicker: () => null }));
vi.mock("../chat/messages/MessageAttachments", () => ({ MessageAttachments: () => null }));

let rows: HomePost[];
let requests: URL[];
let failOlder: boolean;
let prefetchDistances: number[];
let root: HTMLElement;
function makeRouter(initialEntries = ["/home"]) {
  const root = createRootRoute({
    component: () => (
      <LocaleProvider>
        <div style={{ height: 600, display: "flex", flexDirection: "column" }}>
          <Outlet />
        </div>
      </LocaleProvider>
    ),
  });
  const home = createRoute({ getParentRoute: () => root, path: "/home", component: HomeScreen });
  const post = createRoute({
    getParentRoute: () => root,
    path: "/home/posts/$postId",
    component: () => {
      const { postId } = post.useParams();
      return <HomeScreen key={postId} postId={postId} />;
    },
  });
  const profile = createRoute({
    getParentRoute: () => root,
    path: "/home/users/$actorId",
    component: () => {
      const { actorId } = profile.useParams();
      return <HomeScreen key={actorId} actorId={actorId} />;
    },
  });
  const channel = createRoute({
    getParentRoute: () => root,
    path: "/channels/$channelId",
    component: () => <p>直前のチャンネル</p>,
  });
  return createRouter({
    routeTree: root.addChildren([home, post, profile, channel]),
    history: createMemoryHistory({ initialEntries }),
  });
}
let router: ReturnType<typeof makeRouter>;
const view = () => <RouterProvider key={fixture.user} router={router} />;
let failDelete = false;
beforeEach(async () => {
  await page.viewport(1440, 900);
  fixture.user = "me";
  rows = Array.from({ length: 180 }, (_, index) => ({
    id: `p-${index}`,
    user_id: "me",
    user_name: "me",
    display_name: "Me",
    content: `投稿 ${index}` + (index % 3 === 0 ? "\n可変高さの本文".repeat(4) : ""),
    created_at: new Date(1800000000000 - index * 1000).toISOString(),
    sort_key: String(1800000000000000 - index * 1000000),
  }));
  requests = [];
  prefetchDistances = [];
  failOlder = false;
  failDelete = false;
  fixture.get.mockReset();
  fixture.get.mockImplementation(async (path: string) => {
    if (path.includes("/activitypub/status")) return { enabled: true };
    if (path.includes("/activitypub/follows")) return [];
    const url = new URL(path, "https://home.test");
    requests.push(url);
    if (url.searchParams.has("entries")) {
      const ids = JSON.parse(url.searchParams.get("entries")!) as string[];
      return rows.filter((row) => ids.includes(row.id));
    }
    if (url.pathname.endsWith("/thread")) {
      const id = decodeURIComponent(url.pathname.split("/").at(-2)!);
      return rows.filter((row) => row.id === id);
    }
    const before = url.searchParams.get("before");
    if (before) {
      prefetchDistances.push(root.scrollHeight - root.scrollTop - root.clientHeight);
      if (failOlder) throw new Error("offline");
    }
    const start = before ? rows.findIndex((row) => row.id === before) + 1 : 0;
    return rows.slice(start, start + 50);
  });
  fixture.request.mockReset();
  fixture.request.mockImplementation(
    async (path: string, options: { method?: string; body?: { entries: string[] } }) => {
      if (options.method === "DELETE") {
        if (failDelete) throw new Error("delete failed");
        rows = rows.filter((row) => row.id !== decodeURIComponent(path.split("/").at(-1)!));
        return;
      }
      expect(path).toBe("/api/home/entries");
      return fixture.get(
        `/api/home?entries=${encodeURIComponent(JSON.stringify(options.body!.entries))}`,
      );
    },
  );
  router = makeRouter();
  await router.load();
});
async function mount() {
  const rendered = render(view());
  root = rendered.container.querySelector<HTMLElement>("[data-home-scroll]")!;
  await screen.findByText(/^投稿 0/);
  return rendered;
}
function approachTail() {
  fireEvent.scroll(root, { target: { scrollTop: root.scrollHeight - root.clientHeight - 700 } });
}
it("stays on the timeline when dismissing a post media preview", async () => {
  rows[0].media = [
    { url: "https://home.test/clip.mp3", name: "clip.mp3", media_type: "audio/mpeg" },
  ];
  await mount();
  const user = userEvent.setup();
  const preview = screen.getByRole("button", { name: "clip.mp3をプレビュー" });

  await user.click(preview);
  await screen.findByRole("dialog", { name: "clip.mp3" });
  const backdrop = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]')!;
  await user.click(backdrop);
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(router.state.location.pathname).toBe("/home");

  await user.click(preview);
  const reopened = await screen.findByRole("dialog", { name: "clip.mp3" });
  await user.click(within(reopened).getByRole("button", { name: "閉じる" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(router.state.location.pathname).toBe("/home");

  await user.click(screen.getByText(/^投稿 0/));
  await waitFor(() => expect(router.state.location.pathname).toBe("/home/posts/p-0"));
});

it.each([390, 1440])(
  "virtualizes variable height posts and prefetches before the end at width %s",
  async (width) => {
    await page.viewport(width, 900);
    await mount();
    expect(root.querySelectorAll("[data-home-post]").length).toBeLessThan(30);
    for (const count of [1, 2, 3]) {
      approachTail();
      await waitFor(() => expect(prefetchDistances).toHaveLength(count));
      await waitFor(() => expect(screen.queryByRole("button", { name: "読み込み中…" })).toBeNull());
    }
    expect(prefetchDistances.every((distance) => distance > 0)).toBe(true);
    expect(root.querySelectorAll("[data-home-post]").length).toBeLessThan(30);
    root.scrollTop = root.scrollHeight;
    fireEvent.scroll(root);
    await screen.findByText(/^投稿 179/);
    expect(screen.queryByRole("button", { name: "過去の投稿を読む" })).toBeNull();
    expect(
      new Set(
        requests
          .filter((url) => url.searchParams.has("before"))
          .map((url) => url.searchParams.get("before")),
      ).size,
    ).toBe(3);
  },
  15000,
);

it("stops automatic retries after a page error and supports an explicit retry", async () => {
  await mount();
  failOlder = true;
  approachTail();
  await screen.findByRole("alert");
  expect(prefetchDistances).toHaveLength(1);
  failOlder = false;
  fireEvent.click(screen.getByRole("button", { name: "過去の投稿を読む" }));
  await waitFor(() => expect(prefetchDistances).toHaveLength(2));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});

it("refreshes only the head and viewport, applies edits/deletions, and preserves the reading anchor", async () => {
  await mount();
  approachTail();
  await waitFor(() => expect(prefetchDistances).toHaveLength(1));
  await waitFor(() => expect(screen.queryByRole("button", { name: "読み込み中…" })).toBeNull());
  // Enter history beyond the first page, but stay away from the next prefetch boundary.
  root.scrollTop += 3500;
  fireEvent.scroll(root);
  await waitFor(() =>
    expect(
      Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).some(
        (node) => Number(node.dataset.index) >= 60,
      ),
    ).toBe(true),
  );
  // The virtual rows render before their animation-frame ResizeObserver
  // measurements. Capture the reading position after those measurements.
  await act(async () => {
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
  });
  let visible: HTMLElement[] = [];
  await waitFor(() => {
    const viewport = root.getBoundingClientRect();
    visible = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).filter((node) => {
      const rect = node.getBoundingClientRect();
      return rect.bottom > viewport.top && rect.top < viewport.bottom;
    });
    expect(visible.length).toBeGreaterThanOrEqual(2);
  });
  const anchor = visible[0],
    edited = visible[1];
  const deleted = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).find(
    (node) => Number(node.dataset.index) > Number(edited.dataset.index),
  )!;
  const id = edited.dataset.homePost!;
  const anchorId = anchor.dataset.homePost!;
  const deletedId = deleted.dataset.homePost!;
  const top = anchor.getBoundingClientRect().top;
  rows = rows
    .filter((row) => row.id !== deletedId)
    .map((row) => (row.id === id ? { ...row, content: "表示中の編集" } : row));
  rows.unshift({ ...rows[0], id: "new", content: "新着" });
  const pagesBefore = prefetchDistances.length;
  act(() => {
    for (let i = 0; i < 20; i++) fixture.live();
  });
  await screen.findByText("表示中の編集", {}, { timeout: 10000 });
  await waitFor(() => expect(root.querySelector(`[data-home-post="${deletedId}"]`)).toBeNull());
  // Text updates alone do not mean the virtualizer has applied its measured
  // scroll correction. Keep the same tolerance, waiting for that correction.
  await waitFor(() =>
    expect(
      Math.abs(
        root.querySelector(`[data-home-post="${anchorId}"]`)!.getBoundingClientRect().top - top,
      ),
    ).toBeLessThan(5),
  );
  expect(prefetchDistances).toHaveLength(pagesBefore);
  const checks = requests.filter((url) => url.searchParams.has("entries"));
  expect(checks.length).toBeGreaterThan(0);
  expect(checks.every((url) => JSON.parse(url.searchParams.get("entries")!).length <= 100)).toBe(
    true,
  );
  expect(requests.every((url) => !url.searchParams.has("limit"))).toBe(true);
}, 15000);

it("coalesces notifications during a slow response without aborting or overlapping requests", async () => {
  const base = fixture.get.getMockImplementation()!;
  let release: (value: HomePost[]) => void = () => {};
  let headLoads = 0;
  let signal: AbortSignal | undefined;
  fixture.get.mockImplementation(async (path: string, options?: { signal?: AbortSignal }) => {
    if (path === "/api/home?timeline=home") {
      headLoads++;
      if (headLoads === 1) {
        signal = options?.signal;
        return new Promise<HomePost[]>((resolve) => {
          release = resolve;
        });
      }
    }
    return base(path);
  });
  const rendered = render(view());
  root = rendered.container.querySelector<HTMLElement>("[data-home-scroll]")!;
  await waitFor(() => expect(headLoads).toBe(1));
  act(() => {
    for (let i = 0; i < 20; i++) fixture.live();
  });
  expect(headLoads).toBe(1);
  expect(signal?.aborted).toBe(false);
  act(() => release(rows.slice(0, 50)));
  await screen.findByText(/^投稿 0/);
  await waitFor(() => expect(headLoads).toBe(2), { timeout: 10000 });
  expect(signal?.aborted).toBe(false);
}, 15000);

it("aborts an old account request and ignores its late response", async () => {
  const base = fixture.get.getMockImplementation()!;
  let release: (value: HomePost[]) => void = () => {};
  let signal: AbortSignal | undefined;
  let held = false;
  fixture.get.mockImplementation(async (path: string, options?: { signal?: AbortSignal }) => {
    if (path === "/api/home?timeline=home" && !held) {
      held = true;
      signal = options?.signal;
      return new Promise<HomePost[]>((resolve) => {
        release = resolve;
      });
    }
    return base(path);
  });
  const rendered = render(view());
  await waitFor(() => expect(held).toBe(true));
  fixture.user = "other";
  rows = [{ ...rows[0], id: "other-post", content: "切替後の投稿" }];
  rendered.rerender(view());
  await screen.findByText("切替後の投稿");
  expect(signal?.aborted).toBe(true);
  await act(async () => release([{ ...rows[0], id: "old-post", content: "前のアカウント" }]));
  expect(screen.queryByText("前のアカウント")).toBeNull();
  expect(screen.getByText("切替後の投稿")).toBeVisible();
});

async function confirmDeletion(container: HTMLElement) {
  const user = userEvent.setup();
  await user.click(within(container).getByRole("button", { name: "投稿の操作" }));
  await user.click(await screen.findByRole("menuitem", { name: "投稿を削除" }));
  const dialog = await screen.findByRole("alertdialog");
  await user.click(within(dialog).getByRole("button", { name: "投稿を削除" }));
}

it("deletes from the TL menu without navigating or resetting the scroll position", async () => {
  await mount();
  approachTail();
  await waitFor(() => expect(prefetchDistances).toHaveLength(1));
  await waitFor(() => expect(screen.queryByRole("button", { name: "読み込み中…" })).toBeNull());
  // The virtualizer applies row measurements from ResizeObserver on the next frame.
  await act(async () => {
    await document.fonts.ready;
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
  });
  const visible = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).filter(
    (node) => {
      const rect = node.getBoundingClientRect(),
        viewport = root.getBoundingClientRect();
      return rect.bottom > viewport.top && rect.top < viewport.bottom;
    },
  );
  const anchor = visible[0],
    item = visible[1];
  const id = item.dataset.homePost!,
    anchorId = anchor.dataset.homePost!,
    before = anchor.getBoundingClientRect().top;
  await confirmDeletion(item);
  await waitFor(() => expect(root.querySelector(`[data-home-post="${id}"]`)).toBeNull());
  expect(router.state.location.pathname).toBe("/home");
  await waitFor(() =>
    expect(
      Math.abs(
        root.querySelector(`[data-home-post="${anchorId}"]`)!.getBoundingClientRect().top - before,
      ),
    ).toBeLessThan(8),
  );
  expect(root.scrollTop).toBeGreaterThan(1000);
  expect(
    fixture.request.mock.calls.filter(([, options]) => options.method === "DELETE"),
  ).toHaveLength(1);
});

it("restores loaded history and the reading position when deleting a post in its detail page", async () => {
  await mount();
  approachTail();
  await waitFor(() => expect(prefetchDistances).toHaveLength(1));
  await waitFor(() => expect(screen.queryByRole("button", { name: "読み込み中…" })).toBeNull());
  root.scrollTop += 2500;
  fireEvent.scroll(root);
  await waitFor(() =>
    expect(
      Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).some(
        (node) => Number(node.dataset.index) >= 60,
      ),
    ).toBe(true),
  );
  await act(async () => {
    await new Promise(requestAnimationFrame);
    await new Promise(requestAnimationFrame);
  });
  const visible = Array.from(root.querySelectorAll<HTMLElement>("[data-home-post]")).filter(
    (node) => {
      const rect = node.getBoundingClientRect(),
        viewport = root.getBoundingClientRect();
      return rect.bottom > viewport.top && rect.top < viewport.bottom;
    },
  );
  const anchor = visible[0],
    item = visible[1];
  const anchorId = anchor.dataset.homePost!,
    id = item.dataset.homePost!;
  const top = anchor.getBoundingClientRect().top;
  fireEvent.click(item.querySelector("article")!);
  await waitFor(() => expect(router.state.location.pathname).toBe(`/home/posts/${id}`));
  const detail = await screen.findByRole("button", { name: "戻る" });
  await confirmDeletion(detail.closest("section")!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/home"));
  root = document.querySelector<HTMLElement>("[data-home-scroll]")!;
  await waitFor(() => expect(root.querySelector(`[data-home-post="${anchorId}"]`)).not.toBeNull());
  expect(root.querySelector(`[data-home-post="${id}"]`)).toBeNull();
  await waitFor(() =>
    expect(
      Math.abs(
        root.querySelector(`[data-home-post="${anchorId}"]`)!.getBoundingClientRect().top - top,
      ),
    ).toBeLessThan(8),
  );
  expect(root.scrollTop).toBeGreaterThan(1000);
}, 15000);

it.each([false, true])(
  "returns to the preceding screen or safely falls back to Home (previous: %s)",
  async (previous) => {
    router = makeRouter(previous ? ["/channels/general", "/home/posts/p-0"] : ["/home/posts/p-0"]);
    await router.load();
    const rendered = render(view());
    await screen.findByText(/^投稿 0/);
    await confirmDeletion(rendered.container);
    await waitFor(() =>
      expect(router.state.location.pathname).toBe(previous ? "/channels/general" : "/home"),
    );
    if (previous) await screen.findByText("直前のチャンネル");
    else expect(router.history.canGoBack()).toBe(false);
  },
);

it("keeps the detail page and dialog open after a failed deletion, then allows retry", async () => {
  router = makeRouter(["/home", "/home/posts/p-0"]);
  await router.load();
  const rendered = render(view());
  await screen.findByText(/^投稿 0/);
  failDelete = true;
  await confirmDeletion(rendered.container);
  await screen.findByRole("alert");
  expect(router.state.location.pathname).toBe("/home/posts/p-0");
  failDelete = false;
  await userEvent.click(
    within(screen.getByRole("alertdialog")).getByRole("button", { name: "投稿を削除" }),
  );
  await waitFor(() => expect(router.state.location.pathname).toBe("/home"));
});

it("does not retry a failed older page when a live head refresh succeeds", async () => {
  await mount();
  failOlder = true;
  approachTail();
  await screen.findByRole("alert");
  rows.unshift({ ...rows[0], id: "new-after-failure", content: "失敗後の新着" });
  act(() => fixture.live());
  await screen.findByRole("button", { name: "新しい投稿を見る（1件）" }, { timeout: 10000 });
  expect(prefetchDistances).toHaveLength(1);
  expect(screen.getByRole("alert")).toBeVisible();
  failOlder = false;
  fireEvent.click(screen.getByRole("button", { name: "過去の投稿を読む" }));
  await waitFor(() => expect(prefetchDistances).toHaveLength(2));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
}, 15000);

it.each([390, 1440])(
  "reloads timeline tabs from the top while keeping detail back-navigation restoration at width %s",
  async (width) => {
    await page.viewport(width, 900);
    await mount();
    approachTail();
    await waitFor(() => expect(prefetchDistances).toHaveLength(1));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "過去の投稿を読む" })).not.toBeDisabled(),
    );
    expect(root.scrollTop).toBeGreaterThan(1000);
    for (let cycle = 0; cycle < 2; cycle++) {
      await userEvent.click(screen.getByRole("button", { name: "ローカル" }));
      await waitFor(() => expect(root.scrollTop).toBe(0));
      await screen.findByText(/^投稿 0/);
      root.scrollTop = 1200;
      fireEvent.scroll(root);
      await userEvent.click(screen.getByRole("button", { name: "ホーム" }));
      await waitFor(() => expect(root.scrollTop).toBe(0));
      await screen.findByText(/^投稿 0/);
      expect(prefetchDistances).toHaveLength(1);
    }
  },
  15000,
);

it("revalidates a changed history viewport without fetching the head again", async () => {
  await mount();
  approachTail();
  await waitFor(() => expect(prefetchDistances).toHaveLength(1));
  root.scrollTop += 2500;
  fireEvent.scroll(root);
  await waitFor(
    () =>
      expect(fixture.request.mock.calls.some(([path]) => path === "/api/home/entries")).toBe(true),
    { timeout: 10000 },
  );
  const heads = requests.filter(
    (url) => !url.searchParams.has("before") && !url.searchParams.has("entries"),
  );
  expect(heads).toHaveLength(1);
}, 15000);

it("keeps successful head updates and pagination usable when viewport reconciliation fails", async () => {
  await mount();
  approachTail();
  await waitFor(() => expect(prefetchDistances).toHaveLength(1));
  const original = fixture.request.getMockImplementation()!;
  fixture.request.mockImplementation(async (path, options) => {
    if (path === "/api/home/entries") throw new Error("viewport offline");
    return original(path, options);
  });
  root.scrollTop += 2500;
  fireEvent.scroll(root);
  await screen.findByRole("alert", {}, { timeout: 10000 });
  rows.unshift({ ...rows[0], id: "head-despite-viewport", content: "先頭は更新できる" });
  act(() => fixture.live());
  await screen.findByRole("button", { name: "新しい投稿を見る（1件）" }, { timeout: 10000 });
  approachTail();
  await waitFor(() => expect(prefetchDistances).toHaveLength(2), { timeout: 10000 });
}, 20000);

it("applies a disjoint new head when scrolling back to the top", async () => {
  await mount();
  root.scrollTop = 2500;
  fireEvent.scroll(root);
  const incoming = rows.slice(0, 50).map((row, index) => ({
    ...row,
    id: `incoming-${index}`,
    content: `最新の投稿 ${index}`,
    sort_key: String(1900000000000000 - index * 1000000),
  }));
  rows = [...incoming, ...rows];
  act(() => fixture.live());
  await screen.findByRole("button", { name: "新しい投稿を見る（50件）" }, { timeout: 10000 });
  expect(screen.queryByText("最新の投稿 0")).toBeNull();
  root.scrollTop = 0;
  fireEvent.scroll(root);
  await screen.findByText("最新の投稿 0", {}, { timeout: 10000 });
  expect(screen.queryByRole("button", { name: /新しい投稿を見る/ })).toBeNull();
}, 20000);

it.each([390, 1440])(
  "keeps the viewport at the old tail after appending at width %s",
  async (width) => {
    await page.viewport(width, 900);
    await mount();
    root.scrollTop = root.scrollHeight;
    fireEvent.scroll(root);
    await waitFor(() => expect(prefetchDistances).toHaveLength(1));
    await waitFor(() => expect(screen.queryByRole("button", { name: "読み込み中…" })).toBeNull());
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
    expect(prefetchDistances).toHaveLength(1);
    expect(root.scrollHeight - root.scrollTop - root.clientHeight).toBeGreaterThan(1000);
    expect(root.querySelector('[data-home-post="p-49"]')).not.toBeNull();
  },
);

it("shows the original link only inside the post menu", async () => {
  rows[0] = { ...rows[0], remote: true, url: "https://remote.test/notes/first" };
  await mount();
  expect(screen.queryByRole("link", { name: "元の投稿" })).toBeNull();
  fireEvent.click(
    within(root.querySelector('[data-home-post="p-0"]')!).getByRole("button", {
      name: "投稿の操作",
    }),
  );
  const original = await screen.findByRole("menuitem", { name: /元の投稿/ });
  expect(original.getAttribute("href")).toBe("https://remote.test/notes/first");
  expect(original.getAttribute("target")).toBe("_blank");
  expect(original.getAttribute("rel")).toContain("noopener");
});

it.each(["article", "time"])(
  "shows the clicked %s post while its thread is pending, then replaces it with the server thread",
  async (target) => {
    await mount();
    const get = fixture.get.getMockImplementation()!;
    let release!: (posts: HomePost[]) => void;
    let threadRequests = 0;
    fixture.get.mockImplementation((path: string) => {
      if (path.endsWith("/p-0/thread")) {
        threadRequests++;
        return new Promise<HomePost[]>((resolve) => {
          release = resolve;
        });
      }
      return get(path);
    });
    fireEvent.click(root.querySelector(`[data-home-post="p-0"] ${target}`)!);
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/posts/p-0"));
    await screen.findByRole("button", { name: "戻る" });
    expect(screen.getByText(/^投稿 0/)).toBeVisible();
    await waitFor(() => expect(threadRequests).toBe(1));
    await act(async () =>
      release([
        { ...rows[0], content: "更新済みの投稿" },
        { ...rows[1], content: "取得した返信", parent_id: "p-0", tree_depth: 1 },
      ]),
    );
    await screen.findByText("更新済みの投稿");
    await screen.findByText("取得した返信");
    expect(screen.queryByText(/^投稿 0/)).toBeNull();
    expect(threadRequests).toBe(1);
  },
);

it.each([500, 403, 404])("handles a %s response after displaying a known post", async (status) => {
  await mount();
  const get = fixture.get.getMockImplementation()!;
  let reject!: (error: Error) => void;
  fixture.get.mockImplementation((path: string) => {
    if (path.endsWith("/p-0/thread")) {
      return new Promise<HomePost[]>((_resolve, fail) => {
        reject = fail;
      });
    }
    return get(path);
  });
  fireEvent.click(root.querySelector('[data-home-post="p-0"] article')!);
  await screen.findByRole("button", { name: "戻る" });
  expect(screen.getByText(/^投稿 0/)).toBeVisible();
  await waitFor(() => expect(reject).toBeTypeOf("function"));
  await act(async () => reject(Object.assign(new ApiError(status, "unavailable"), { status })));
  await screen.findByRole("alert");
  if (status === 500) expect(screen.getByText(/^投稿 0/)).toBeVisible();
  else expect(screen.queryByText(/^投稿 0/)).toBeNull();
});
