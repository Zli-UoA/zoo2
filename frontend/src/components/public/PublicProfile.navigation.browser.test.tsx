import "../../styles.css";
import { act, useEffect } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { http, HttpResponse } from "msw";
import { mockServer, startMockServer } from "../../test/browser-setup";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { publicProfile } from "../../../.storybook/public-profile-fixtures";
import { PublicProfile } from "./PublicProfile";
import { RouteAccess } from "../auth/RouteAccess";
import { page } from "vite-plus/test/browser/context";

const auth = vi.hoisted(() => ({
  client: {
    hasSession: true,
    isCallbackUrl: () => false,
    getAccessToken: vi.fn(async () => "token"),
    setReauthorizeHandler: vi.fn(),
    resetSession: vi.fn(),
    login: vi.fn(async () => "https://id.example/authorize"),
  },
  getClient: vi.fn(),
}));
vi.mock("../../lib/oidc/session", () => ({
  getOidcClient: () => {
    auth.getClient();
    return auth.client;
  },
}));
vi.mock("../../lib/config", async (original) => ({
  ...(await original<typeof import("../../lib/config")>()),
  resolveConfig: () => ({ oidc: { issuer: "https://id.example", clientId: "zoo" } }),
}));
beforeEach(async () => {
  await page.viewport(390, 600);
  sessionStorage.clear();
  auth.client.login.mockClear();
  auth.getClient.mockClear();
});

it.each([true, false])(
  "uses the actual auth boundary for internal links (authenticated: %s)",
  async (authenticated) => {
    auth.client.hasSession = authenticated;
    await startMockServer();
    mockServer.use(
      http.get("*/api/activitypub/users/alice/profile", ({ request }) =>
        HttpResponse.json(
          new URL(request.url).searchParams.has("before")
            ? { ...publicProfile, posts: [], next_cursor: null }
            : publicProfile,
        ),
      ),
    );
    const mount = vi.fn();
    const unmount = vi.fn();
    const root = createRootRoute({
      component: () => {
        useEffect(() => {
          mount();
          return unmount;
        }, []);
        return (
          <LocaleProvider>
            <RouteAccess>
              <Outlet />
            </RouteAccess>
          </LocaleProvider>
        );
      },
    });
    const profile = createRoute({
      getParentRoute: () => root,
      path: "/public/users/$userId",
      validateSearch: (search: Record<string, unknown>): { before?: string } => ({
        before: typeof search.before === "string" ? search.before : undefined,
      }),
      component: () => {
        const { userId } = profile.useParams();
        const { before } = profile.useSearch();
        return <PublicProfile userId={userId} before={before} />;
      },
    });
    const home = createRoute({
      getParentRoute: () => root,
      path: "/",
      component: () => <p>Home</p>,
    });
    const user = createRoute({
      getParentRoute: () => root,
      path: "/home/users/$actorId",
      component: () => <p>Internal profile</p>,
    });
    const history = createMemoryHistory({ initialEntries: ["/public/users/alice"] });
    const router = createRouter({ routeTree: root.addChildren([profile, home, user]), history });
    await router.load();
    render(<RouterProvider router={router} />);
    const click = userEvent.setup();
    const older = await screen.findByRole("link", { name: "過去の投稿" });
    const main = screen.getByRole("main");
    main.scrollTop = main.scrollHeight;
    expect(main.scrollTop).toBeGreaterThan(0);
    await click.click(older);
    await screen.findByText("公開の投稿はまだありません。");
    expect(main.scrollTop).toBe(0);
    expect(router.state.location.search).toEqual({ before: "p1" });
    await act(async () => {
      history.back();
    });
    await screen.findByRole("link", { name: "過去の投稿" });
    expect(router.state.location.search).toEqual({});
    await act(async () => {
      history.forward();
    });
    await screen.findByText("公開の投稿はまだありません。");
    await click.click(screen.getByRole("link", { name: "最新の投稿" }));
    await screen.findByRole("link", { name: "過去の投稿" });
    expect(auth.getClient).not.toHaveBeenCalled();
    await click.click(screen.getByRole("link", { name: "Zooでプロフィールを開く" }));
    if (!authenticated) {
      await waitFor(() => expect(auth.client.login).toHaveBeenCalledOnce());
      expect(router.state.location.pathname).toBe("/home/users/alice");
      expect(mount).toHaveBeenCalledTimes(1);
      expect(unmount).not.toHaveBeenCalled();
      return;
    }
    await screen.findByText("Internal profile");
    expect(auth.getClient).toHaveBeenCalled();
    expect(auth.client.login).not.toHaveBeenCalled();
    await act(async () => {
      history.back();
    });
    await screen.findByRole("link", { name: "過去の投稿" });
    await click.click(screen.getByRole("link", { name: "Zoo" }));
    await screen.findByText("Home");
    await waitFor(() => expect(mount).toHaveBeenCalledTimes(1));
    expect(unmount).not.toHaveBeenCalled();
  },
);
