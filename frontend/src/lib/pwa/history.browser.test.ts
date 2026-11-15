import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { createElement } from "react";
import { render, cleanup, waitFor } from "@testing-library/react";
import { createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { createAppHistory, PWA_HISTORY_SESSION_KEY } from "./history";

let appHistory: ReturnType<typeof createAppHistory> | undefined;
let originalUrl: string;
let originalState: unknown;

beforeEach(() => {
  window.sessionStorage.removeItem(PWA_HISTORY_SESSION_KEY);
  originalUrl = window.location.href;
  originalState = window.history.state;
  window.history.replaceState(
    { launchData: "preserved" },
    "",
    `${window.location.pathname}?launch#post`,
  );
});
afterEach(() => {
  cleanup();
  appHistory?.destroy();
  appHistory = undefined;
  window.sessionStorage.removeItem(PWA_HISTORY_SESSION_KEY);
  vi.restoreAllMocks();
  window.history.replaceState(originalState, "", originalUrl);
});

function setStandalone(enabled: boolean) {
  const original = window.matchMedia.bind(window);
  vi.spyOn(window, "matchMedia").mockImplementation((query) => {
    const result = original(query);
    if (query === "(display-mode: standalone)") {
      vi.spyOn(result, "matches", "get").mockReturnValue(enabled);
    }
    return result;
  });
}

async function traverse(action: () => void, events = 1) {
  if (events === 2) {
    const key = window.history.state.__TSR_key;
    const forward = vi.spyOn(window.history, "forward");
    action();
    await vi.waitFor(() => expect(forward).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(window.history.state.__TSR_key).toBe(key));
    forward.mockRestore();
    return;
  }
  await new Promise<void>((resolve) => {
    const listener = () => {
      if (--events > 0) return;
      window.removeEventListener("popstate", listener);
      resolve();
    };
    window.addEventListener("popstate", listener);
    action();
  });
}

it("absorbs repeated back at the PWA boundary while preserving forward history", async () => {
  setStandalone(true);
  const startUrl = window.location.href;
  const length = window.history.length;
  appHistory = createAppHistory();
  expect(window.history.length).toBe(length + 1);
  expect(window.location.href).toBe(startUrl);
  expect(appHistory.canGoBack()).toBe(false);
  expect(appHistory.location.state).toMatchObject({ launchData: "preserved" });

  const boundaryVisited = vi.fn();
  appHistory.subscribe(({ location }) => {
    if (location.state.__TSR_index < 0) boundaryVisited();
  });
  appHistory.push(`${window.location.pathname}?thread=one`);
  await Promise.resolve();
  appHistory.flush();
  expect(appHistory.canGoBack()).toBe(true);
  await traverse(() => window.history.back());
  expect(window.location.href).toBe(startUrl);
  expect(appHistory.canGoBack()).toBe(false);

  for (let index = 0; index < 2; index++) {
    await traverse(() => window.history.back(), 2);
    expect(window.location.href).toBe(startUrl);
    expect(appHistory.location.state.__TSR_index).toBe(0);
    expect(appHistory.canGoBack()).toBe(false);
  }
  await traverse(() => window.history.forward());
  expect(appHistory.location.href).toBe(`${window.location.pathname}?thread=one`);
  expect(boundaryVisited).not.toHaveBeenCalled();
});

it("preserves the boundary and forward entries when history is recreated", async () => {
  setStandalone(true);
  appHistory = createAppHistory();
  const first = appHistory.location.href;
  appHistory.push(`${window.location.pathname}?screen=settings`);
  await Promise.resolve();
  appHistory.flush();
  appHistory.replace(`${window.location.pathname}?screen=notifications`, { custom: "kept" });
  await Promise.resolve();
  appHistory.flush();
  const length = window.history.length;
  const key = window.history.state.__TSR_key;
  appHistory.destroy();
  appHistory = createAppHistory();
  expect(window.history.length).toBe(length);
  expect(window.history.state.__TSR_key).toBe(key);
  expect(appHistory.location.state).toMatchObject({ custom: "kept" });
  await traverse(() => appHistory!.back());
  expect(appHistory.location.href).toBe(first);

  appHistory.destroy();
  appHistory = createAppHistory();
  expect(window.history.length).toBe(length);
  await traverse(() => window.history.back(), 2);
  expect(appHistory.location.href).toBe(first);
  await traverse(() => appHistory!.forward());
  expect(appHistory.location.href).toBe(`${window.location.pathname}?screen=notifications`);
});

it("leaves normal browser-tab history unchanged", async () => {
  setStandalone(false);
  const before = window.location.href;
  window.history.pushState({}, "", `${window.location.pathname}?normal-tab`);
  const length = window.history.length;
  appHistory = createAppHistory();
  expect(window.history.length).toBe(length);
  expect(window.history.state.zooPwaHistory).toBeUndefined();
  await traverse(() => window.history.back());
  expect(window.location.href).toBe(before);
});

it("recognizes iOS home-screen standalone mode", async () => {
  setStandalone(false);
  Object.defineProperty(window.navigator, "standalone", { value: true, configurable: true });
  try {
    appHistory = createAppHistory();
    expect(appHistory.canGoBack()).toBe(false);
    await traverse(() => window.history.back(), 2);
    expect(appHistory.location.state.__TSR_index).toBe(0);
  } finally {
    Reflect.deleteProperty(window.navigator, "standalone");
  }
});

it("preserves forward history on same-URL router navigation", async () => {
  setStandalone(true);
  window.history.replaceState({}, "", "/settings");
  appHistory = createAppHistory();
  const root = createRootRoute();
  const settings = createRoute({
    getParentRoute: () => root,
    path: "/settings",
    component: () => createElement("p", null, "Settings"),
  });
  const home = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => createElement("p", null, "Home"),
  });
  const router = createRouter({
    routeTree: root.addChildren([settings, home]),
    history: appHistory,
  });
  await router.load();
  render(createElement(RouterProvider, { router }));
  await router.navigate({ to: "/" });
  await traverse(() => window.history.back());
  await waitFor(() => expect(router.state.location.pathname).toBe("/settings"));
  const length = window.history.length;
  const key = window.history.state.__TSR_key;
  await router.navigate({ to: "/settings" });
  expect(window.history.length).toBe(length);
  expect(window.history.state.__TSR_key).toBe(key);
  await traverse(() => window.history.forward());
  await waitFor(() => expect(router.state.location.pathname).toBe("/"));
});

it("keeps sequential push and replace indexes synchronous", async () => {
  setStandalone(true);
  appHistory = createAppHistory();
  const first = appHistory.location.href;
  appHistory.push(`${window.location.pathname}?a`);
  appHistory.replace(`${window.location.pathname}?b`);
  appHistory.flush();
  expect(appHistory.location.state.__TSR_index).toBe(1);
  expect(appHistory.canGoBack()).toBe(true);
  await traverse(() => window.history.back());
  expect(appHistory.location.href).toBe(first);
});

it("does not add a boundary after a new notification or callback document", async () => {
  setStandalone(true);
  appHistory = createAppHistory();
  const first = appHistory.location.href;
  appHistory.destroy();
  window.history.pushState(null, "", `${window.location.pathname}?notification`);
  const length = window.history.length;
  appHistory = createAppHistory();
  expect(window.history.length).toBe(length);
  expect(appHistory.canGoBack()).toBe(true);
  await traverse(() => window.history.back());
  expect(appHistory.location.href).toBe(first);
});

it("moves forward when the boundary document is restored after eviction", async () => {
  setStandalone(true);
  appHistory = createAppHistory();
  const first = appHistory.location.href;
  const length = window.history.length;
  appHistory.destroy();
  await traverse(() => window.history.back());
  await traverse(() => {
    appHistory = createAppHistory();
  });
  expect(window.history.length).toBe(length);
  expect(appHistory!.location.href).toBe(first);
  await vi.waitFor(() => expect(appHistory!.location.state.__TSR_index).toBe(0));
});

it("leaves native navigation available when session storage is unavailable", () => {
  setStandalone(true);
  const length = window.history.length;
  vi.spyOn(window.sessionStorage, "getItem").mockImplementation(() => {
    throw new DOMException("Denied", "SecurityError");
  });
  appHistory = createAppHistory();
  expect(window.history.length).toBe(length);
});
