// @vitest-environment happy-dom
import { createElement } from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { AppConsent } from "./AppConsent";

const fixture = vi.hoisted(() => ({
  config: {},
  request:
    vi.fn<(path: string, options: { signal?: AbortSignal }) => Promise<{ redirect_uri: string }>>(),
}));
vi.mock("../../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: fixture.config }),
}));
vi.mock("../../hooks/useRealtime", () => ({ useAllUsers: () => ({ data: [] }) }));
vi.mock("../../lib/api/client", () => ({
  createConfiguredApiClient: () => ({
    get: async () => ({
      name: "Test app",
      manifest: { scopes: ["messages:read"], redirect_uris: ["https://example.com/callback"] },
    }),
    request: fixture.request,
  }),
}));

beforeEach(() => {
  localStorage.setItem("zoo-locale", "ja");
  fixture.request.mockReset();
  window.history.replaceState(
    null,
    "",
    "/apps-authorize?app_id=app&scope=messages%3Aread&redirect_uri=https%3A%2F%2Fexample.com%2Fcallback&code_challenge_method=S256&state=state&code_challenge=challenge",
  );
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

async function setup() {
  const root = createRootRoute();
  const consent = createRoute({
    getParentRoute: () => root,
    path: "/apps-authorize",
    component: AppConsent,
  });
  const home = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => createElement("p", null, "Home"),
  });
  const router = createRouter({
    routeTree: root.addChildren([consent, home]),
    history: createMemoryHistory({ initialEntries: ["/", "/apps-authorize"] }),
  });
  await router.load();
  render(createElement(LocaleProvider, null, createElement(RouterProvider, { router })));
  await screen.findByRole("heading", { name: "Test app" });
  return router;
}

it.each(["cancel", "back"])(
  "does not redirect on a late authorization response after %s",
  async (action) => {
    let resolve!: (result: { redirect_uri: string }) => void;
    fixture.request.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => {});
    const router = await setup();
    await userEvent.click(screen.getByRole("button", { name: "許可する" }));
    expect(fixture.request).toHaveBeenCalledOnce();
    if (action === "cancel")
      await userEvent.click(screen.getByRole("link", { name: "キャンセル" }));
    else
      await act(async () => {
        router.history.back();
      });
    await screen.findByText("Home");
    expect(fixture.request.mock.calls[0][1].signal?.aborted).toBe(true);
    await act(async () => {
      resolve({ redirect_uri: "https://example.com/callback?code=late" });
    });
    expect(assign).not.toHaveBeenCalled();
  },
);

it("redirects on a successful authorization while still on the consent screen", async () => {
  fixture.request.mockResolvedValue({ redirect_uri: "https://example.com/callback?code=ok" });
  const assign = vi.spyOn(window.location, "assign").mockImplementation(() => {});
  await setup();
  await userEvent.click(screen.getByRole("button", { name: "許可する" }));
  await waitFor(() => expect(assign).toHaveBeenCalledWith("https://example.com/callback?code=ok"));
});
