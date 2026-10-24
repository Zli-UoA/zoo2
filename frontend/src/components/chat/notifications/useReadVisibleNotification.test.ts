// @vitest-environment happy-dom
import { act, createElement, createRef } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vite-plus/test";
import { useReadVisibleNotification } from "./useReadVisibleNotification";

it("reads only intersecting cards in a focused visible tab and disconnects on unmount", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const callbacks: ((entries: { isIntersecting: boolean }[]) => void)[] = [];
  const disconnect = vi.fn();
  const observe = vi.fn();
  const observer = vi.fn(
    class {
      constructor(
        callback: (entries: { isIntersecting: boolean }[]) => void,
        _options?: IntersectionObserverInit,
      ) {
        callbacks.push(callback);
      }
      observe = observe;
      disconnect = disconnect;
    },
  );
  vi.stubGlobal("IntersectionObserver", observer);
  const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const focus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
  const scrollRoot = createRef<HTMLDivElement>();
  scrollRoot.current = document.createElement("div");
  const onRead = vi.fn();
  const root = createRoot(document.createElement("div"));
  function Probe({ ids }: { ids: readonly string[] }) {
    const { ref } = useReadVisibleNotification(ids, onRead, scrollRoot);
    return createElement("li", { ref });
  }
  try {
    await act(async () => root.render(createElement(Probe, { ids: ["visible-1", "visible-2"] })));
    expect(observer.mock.calls[0][1]).toMatchObject({ root: scrollRoot.current });
    await act(async () => callbacks.at(-1)!([{ isIntersecting: false }]));
    expect(onRead).not.toHaveBeenCalled();
    visibility.mockReturnValue("hidden");
    await act(async () => callbacks.at(-1)!([{ isIntersecting: true }]));
    expect(onRead).not.toHaveBeenCalled();
    visibility.mockReturnValue("visible");
    focus.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(onRead).not.toHaveBeenCalled();
    focus.mockReturnValue(true);
    window.dispatchEvent(new Event("focus"));
    expect(onRead).toHaveBeenLastCalledWith(["visible-1", "visible-2"]);
    await act(async () => root.render(createElement(Probe, { ids: ["new-reaction"] })));
    await act(async () => callbacks.at(-1)!([{ isIntersecting: true }]));
    expect(onRead).toHaveBeenLastCalledWith(["new-reaction"]);
    onRead.mockClear();
    await act(async () => root.unmount());
    document.dispatchEvent(new Event("visibilitychange"));
    expect(onRead).not.toHaveBeenCalled();
    expect(disconnect).toHaveBeenCalled();
  } finally {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it("loads an already-read card only after it becomes visible, without acknowledging anything", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let callback!: (entries: { isIntersecting: boolean }[]) => void;
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(notify: typeof callback) {
        callback = notify;
      }
      observe() {}
      disconnect() {}
    },
  );
  const root = createRoot(document.createElement("div"));
  const onRead = vi.fn();
  const scrollRoot = createRef<HTMLDivElement>();
  let seen = false;
  const ids: string[] = [];
  function Probe() {
    const result = useReadVisibleNotification(ids, onRead, scrollRoot);
    seen = result.hasBeenVisible;
    return createElement("li", { ref: result.ref });
  }
  try {
    await act(async () => root.render(createElement(Probe)));
    expect(seen).toBe(false);
    await act(async () => callback([{ isIntersecting: false }]));
    expect(seen).toBe(false);
    await act(async () => callback([{ isIntersecting: true }]));
    expect(seen).toBe(true);
    expect(onRead).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    vi.unstubAllGlobals();
  }
});
