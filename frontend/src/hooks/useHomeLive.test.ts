// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { HomeLiveProvider, useHomeLive } from "./useHomeLive";

const mocks = vi.hoisted(() => ({ config: { apiBaseUrl: "https://zoo.test", devUserId: "me" } }));
vi.mock("../lib/electric/collections-context", () => ({
  useCollections: () => ({ config: mocks.config }),
}));
vi.mock("../lib/api/client", () => ({
  buildAuthHeaders: async () => new Headers({ "x-dev-user": "me" }),
}));
class Socket {
  static CLOSED = 3;
  static instances: Socket[] = [];
  readyState = 1;
  onopen?: () => void;
  onmessage?: (event: { data: string }) => void;
  onclose?: () => void;
  send = vi.fn();
  close = vi.fn(() => {
    this.readyState = Socket.CLOSED;
    this.onclose?.();
  });
  constructor(readonly url: string | URL) {
    Socket.instances.push(this);
  }
}
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("WebSocket", Socket);
  vi.stubGlobal("location", new URL("https://zoo.test/"));
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  vi.useFakeTimers();
  Socket.instances = [];
  root = createRoot(document.createElement("div"));
});
afterEach(async () => {
  await act(async () => root.unmount());
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
function Probe({ refresh }: { refresh: () => void }) {
  useHomeLive(refresh);
  return null;
}
function tree(first: () => void, second?: () => void) {
  return createElement(HomeLiveProvider, {
    children: [
      createElement(Probe, { key: "first", refresh: first }),
      second ? createElement(Probe, { key: "second", refresh: second }) : null,
    ],
  });
}
it("shares one authenticated socket and coalesces bursts without reconnecting for consumers", async () => {
  const first = vi.fn(),
    second = vi.fn();
  await act(async () => root.render(tree(first, second)));
  expect(Socket.instances).toHaveLength(1);
  const socket = Socket.instances[0];
  expect(String(socket.url)).toBe("wss://zoo.test/api/home/live");
  socket.onopen?.();
  expect(socket.send).toHaveBeenCalledWith(JSON.stringify({ "x-dev-user": "me" }));
  await act(async () => {
    socket.onmessage?.({ data: "refresh" });
    for (let event = 0; event < 20; event++) socket.onmessage?.({ data: "refresh" });
  });
  expect(first).toHaveBeenCalledTimes(1);
  expect(second).toHaveBeenCalledTimes(1);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(first).toHaveBeenCalledTimes(2);
  expect(second).toHaveBeenCalledTimes(2);
  await act(async () => root.render(tree(first)));
  expect(socket.close).not.toHaveBeenCalled();
  await act(async () => {
    socket.onmessage?.({ data: "refresh" });
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(first).toHaveBeenCalledTimes(3);
  expect(second).toHaveBeenCalledTimes(2);
  await act(async () => root.render(null));
  expect(socket.close).toHaveBeenCalledTimes(1);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(20000);
  });
  expect(Socket.instances).toHaveLength(1);
});
it("defers hidden-tab updates until visible and reconnects a closed connection", async () => {
  const refresh = vi.fn();
  await act(async () => root.render(tree(refresh)));
  const socket = Socket.instances[0];
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  await act(async () => {
    for (let event = 0; event < 20; event++) socket.onmessage?.({ data: "refresh" });
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(refresh).not.toHaveBeenCalled();
  socket.close();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(Socket.instances).toHaveLength(2);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(20000);
  });
  expect(Socket.instances).toHaveLength(2);
});
