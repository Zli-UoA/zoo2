import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
  useRouterState,
} from "@tanstack/react-router";
import { afterEach, describe, expect, it, vi } from "vite-plus/test";
import { MfmContent } from "./MfmContent";

const origin = window.location.origin;

afterEach(() => {
  vi.restoreAllMocks();
});

/** チャンネル画面のルートと同じ検索パラメーターを受け取る、本文だけの小さなアプリ。 */
function setup(content: string) {
  const root = createRootRoute({ component: Outlet });
  const home = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => <MfmContent content={content} />,
  });
  const channel = createRoute({
    getParentRoute: () => root,
    path: "/channels/$channelId",
    validateSearch: (search: Record<string, unknown>): { message?: string; thread?: string } => ({
      message: typeof search.message === "string" ? search.message : undefined,
      thread: typeof search.thread === "string" ? search.thread : undefined,
    }),
    component: function Channel() {
      const location = useRouterState({ select: (state) => state.location });
      const { message, thread } = location.search as { message?: string; thread?: string };
      const channelId = location.pathname.split("/").at(-1);
      return (
        <output aria-label="Opened">{[channelId, message ?? "", thread ?? ""].join("|")}</output>
      );
    },
  });
  const router = createRouter({
    routeTree: root.addChildren([home, channel]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

/** 実際のブラウザー遷移を防ぎつつ、ルーターが既定の動作を止めたか（同じタブで処理したか）を記録する。 */
function observeDefaultPrevented() {
  const prevented: boolean[] = [];
  const observe = (event: MouseEvent) => {
    prevented.push(event.defaultPrevented);
    event.preventDefault();
  };
  document.addEventListener("click", observe);
  return { prevented, stop: () => document.removeEventListener("click", observe) };
}

describe("in-app links in message text", () => {
  it.each([
    ["a message link", `${origin}/channels/c1?message=m1`, "/channels/c1?message=m1", "c1|m1|"],
    [
      "a thread reply link",
      `${origin}/channels/c1?thread=root&message=m2`,
      "/channels/c1?thread=root&message=m2",
      "c1|m2|root",
    ],
    ["a channel link", `${origin}/channels/c2`, "/channels/c2", "c2||"],
  ])("opens %s in the same tab", async (_name, url, href, opened) => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    const router = setup(`見てください ${url}`);
    const link = await screen.findByRole("link");
    expect(link.getAttribute("target")).toBeNull();
    expect(link.getAttribute("href")).toBe(href);

    const clicks = observeDefaultPrevented();
    try {
      await userEvent.click(link);
    } finally {
      clicks.stop();
    }
    // ブラウザー既定のページ遷移は止め、ルーターが同じタブで移動する。
    expect(clicks.prevented).toEqual([true]);
    await waitFor(() => expect(screen.getByLabelText("Opened").textContent).toBe(opened));
    expect(router.state.location.href).toBe(href);
    expect(open).not.toHaveBeenCalled();
  });

  it("opens a labelled markdown link in the same tab", async () => {
    setup(`[議論のスレッド](${origin}/channels/c3?message=m3)`);
    await userEvent.click(await screen.findByRole("link", { name: "議論のスレッド" }));
    await waitFor(() => expect(screen.getByLabelText("Opened").textContent).toBe("c3|m3|"));
  });

  it.each([
    ["Ctrl", { ctrlKey: true }],
    ["Meta", { metaKey: true }],
    ["Shift", { shiftKey: true }],
    ["Alt", { altKey: true }],
  ])("leaves %s-click to the browser so the link can still open elsewhere", async (_name, keys) => {
    const router = setup(`${origin}/channels/c1?message=m1`);
    const link = await screen.findByRole("link");
    const clicks = observeDefaultPrevented();
    try {
      fireEvent.click(link, { button: 0, ...keys });
    } finally {
      clicks.stop();
    }
    // ルーターは既定の動作を止めない。移動はブラウザー（別タブ・別ウィンドウ）に任せる。
    expect(clicks.prevented).toEqual([false]);
    expect(router.state.location.href).toBe("/");
  });

  it.each([
    ["another site", "https://example.com/channels/c1?message=m1"],
    ["another origin with the same path", `https://other.${window.location.host}/channels/c1`],
    ["a same-origin page that is not a channel", `${origin}/settings`],
    ["a same-origin API address", `${origin}/api/channels/c1`],
    ["an address with credentials", `https://user:pass@${window.location.host}/channels/c1`],
  ])("keeps %s as an external link in a new tab", async (_name, url) => {
    setup(`${url}`);
    const link = await screen.findByRole("link");
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    expect(link.getAttribute("href")).toBe(url);
  });

  it("falls back to an external link outside a router without warning", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    render(<MfmContent content={`${origin}/channels/c1?message=m1`} />);
    const link = await screen.findByRole("link");
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.getAttribute("href")).toBe(`${origin}/channels/c1?message=m1`);
    expect(warn).not.toHaveBeenCalled();
  });
});
