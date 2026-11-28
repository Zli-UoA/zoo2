import { render, screen } from "@testing-library/react";
import {
  Outlet,
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { describe, expect, it } from "vite-plus/test";

import { LocaleProvider } from "../i18n/LocaleProvider";
import { routeTree } from "../../routeTree.gen";
import { NotFoundScreen } from "./NotFoundScreen";

function setup(path: string, locale: string) {
  localStorage.setItem("zoo-locale", locale);
  const root = createRootRoute({
    component: () => (
      <div data-testid="app-shell">
        <Outlet />
      </div>
    ),
  });
  const splat = createRoute({ getParentRoute: () => root, path: "$", component: NotFoundScreen });
  const index = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => <p>home</p>,
  });
  const router = createRouter({
    routeTree: root.addChildren([splat, index]),
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(
    <LocaleProvider>
      <RouterProvider router={router} />
    </LocaleProvider>,
  );
  return router;
}

describe("NotFoundScreen", () => {
  it("shows a localized 404 with a link back to channels inside the shell", async () => {
    setup("/this-route-does-not-exist", "ja");
    expect(
      await screen.findByRole("heading", { level: 1, name: "ページが見つかりません" }),
    ).toBeVisible();
    const back = screen.getByRole("link", { name: "チャンネル一覧へ戻る" });
    expect(back).toHaveAttribute("href", "/");
    expect(back.closest("[data-testid='app-shell']")).not.toBeNull();
    expect(screen.queryByText("Not Found")).toBeNull();
  });

  it("uses the active locale", async () => {
    setup("/missing", "en");
    expect(await screen.findByRole("heading", { level: 1, name: "Page not found" })).toBeVisible();
    expect(screen.getByRole("link", { name: "Back to channels" })).toBeVisible();
  });
});

describe("route tree", () => {
  it("registers the catch-all under the shell layout", () => {
    type RouteNode = { options?: { path?: string }; children?: RouteNode[] };
    const shell = (routeTree as RouteNode).children?.find(
      (route) => route.options?.path === undefined,
    );
    expect(shell?.children?.some((route) => route.options?.path === "/$")).toBe(true);
  });
});
