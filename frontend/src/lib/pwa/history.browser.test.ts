import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { createAppHistory } from "./history";

let appHistory: ReturnType<typeof createAppHistory> | undefined;
let originalUrl: string;
let originalState: unknown;

beforeEach(() => {
  originalUrl = window.location.href;
  originalState = window.history.state;
  window.history.replaceState(
    { launchData: "preserved" },
    "",
    `${window.location.pathname}?launch#post`,
  );
});
afterEach(() => {
  appHistory?.destroy();
  appHistory = undefined;
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
    if (location.state.zooPwaBoundary) boundaryVisited();
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
    expect(appHistory.location.state.zooPwaBoundary).toBe(false);
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
    expect(appHistory.location.state.zooPwaBoundary).toBe(false);
  } finally {
    Reflect.deleteProperty(window.navigator, "standalone");
  }
});
