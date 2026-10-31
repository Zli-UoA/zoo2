import { afterEach, expect, it, vi } from "vite-plus/test";
import { startAssetCaching } from "./cache-assets";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
function setup() {
  vi.useFakeTimers();
  const postMessage = vi.fn();
  const serviceWorker = Object.assign(new EventTarget(), {
    controller: { postMessage },
    ready: Promise.resolve(),
  });
  const entries = [{ name: "https://zoo.example/assets/app.js" }];
  let receive!: (list: { getEntries: () => PerformanceEntry[] }) => void;
  const disconnect = vi.fn();
  vi.stubGlobal(
    "window",
    Object.assign(new EventTarget(), { location: { origin: "https://zoo.example" } }),
  );
  vi.stubGlobal("navigator", { serviceWorker });
  vi.stubGlobal("performance", { getEntriesByType: () => entries });
  vi.stubGlobal(
    "PerformanceObserver",
    class {
      constructor(callback: typeof receive) {
        receive = callback;
      }
      observe() {}
      disconnect = disconnect;
    },
  );
  const stop = startAssetCaching();
  return {
    postMessage,
    serviceWorker,
    stop,
    disconnect,
    observe: (...names: string[]) =>
      receive({ getEntries: () => names.map((name) => ({ name })) as PerformanceEntry[] }),
  };
}
it("sends new static assets only, ignoring repeated entries and API/Shape polling", async () => {
  const { postMessage, observe, stop } = setup();
  await vi.advanceTimersByTimeAsync(1_000);
  expect(postMessage).toHaveBeenCalledExactlyOnceWith({
    type: "CACHE_ASSETS",
    urls: ["/assets/app.js"],
  });
  observe(
    "https://zoo.example/api/messages",
    "https://zoo.example/api/shapes/messages",
    "https://zoo.example/assets/app.js",
  );
  await vi.advanceTimersByTimeAsync(4_000);
  expect(postMessage).toHaveBeenCalledTimes(1);
  observe("https://zoo.example/compose/font.ttf");
  await vi.advanceTimersByTimeAsync(300);
  expect(postMessage).toHaveBeenLastCalledWith({
    type: "CACHE_ASSETS",
    urls: ["/compose/font.ttf"],
  });
  stop();
});
it("resends known assets to a replacement controller", async () => {
  const { serviceWorker, postMessage, stop } = setup();
  await vi.advanceTimersByTimeAsync(1_000);
  const next = vi.fn();
  serviceWorker.controller = { postMessage: next };
  serviceWorker.dispatchEvent(new Event("controllerchange"));
  expect(next).toHaveBeenCalledExactlyOnceWith({ type: "CACHE_ASSETS", urls: ["/assets/app.js"] });
  expect(postMessage).toHaveBeenCalledTimes(1);
  stop();
});
it("disconnects and cancels pending notifications on unmount", async () => {
  const { postMessage, observe, stop, disconnect } = setup();
  observe("https://zoo.example/compose/late.wasm");
  stop();
  await vi.advanceTimersByTimeAsync(5_000);
  expect(disconnect).toHaveBeenCalledTimes(1);
  expect(postMessage).not.toHaveBeenCalled();
});
