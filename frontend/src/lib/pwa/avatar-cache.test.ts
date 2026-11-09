import { afterEach, expect, it, vi } from "vite-plus/test";

const stops: (() => void)[] = [];
afterEach(() => {
  stops.splice(0).forEach((stop) => stop());
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.resetModules();
});
const url = "https://avater.zliusercontent.com/alice.png";
async function setup() {
  vi.useFakeTimers();
  const controller = { postMessage: vi.fn() };
  const workers = Object.assign(new EventTarget(), { controller });
  const images = [
    {
      src: url,
      complete: true,
      naturalWidth: 32,
      getAttribute() {
        return this.src;
      },
      parentElement: { getBoundingClientRect: () => ({ top: 0, bottom: 32, left: 0, right: 32 }) },
    },
  ];
  const document = Object.assign(new EventTarget(), {
    visibilityState: "visible",
    querySelectorAll: () => images,
  });
  const window = Object.assign(new EventTarget(), {
    innerHeight: 800,
    innerWidth: 400,
    setInterval,
    clearInterval,
  });
  const navigator = { onLine: true, serviceWorker: workers };
  vi.stubGlobal("navigator", navigator);
  vi.stubGlobal("window", window);
  vi.stubGlobal("document", document);
  return {
    controller,
    workers,
    images,
    document,
    window,
    navigator,
    ...(await import("./avatar-cache")),
  };
}
function message(workers: EventTarget, source: unknown, target = url, revision = "a".repeat(64)) {
  workers.dispatchEvent(
    Object.assign(new Event("message"), {
      source,
      data: { type: "AVATAR_UPDATED", url: target, revision },
    }),
  );
}

it("updates the matching image source only for a valid message from the controlling SW", async () => {
  const app = await setup();
  const alice = vi.fn(),
    bob = vi.fn();
  stops.push(
    app.subscribeAvatar(url, alice),
    app.subscribeAvatar("https://avater.zliusercontent.com/bob.png", bob),
  );
  message(app.workers, {}, url);
  message(app.workers, app.controller, "https://other.example/avatar.png");
  message(app.workers, app.controller, url, "invalid");
  expect(alice).not.toHaveBeenCalled();
  expect(app.avatarSource(url)).toBe(url);
  message(app.workers, app.controller);
  expect(alice).toHaveBeenCalledTimes(1);
  expect(bob).not.toHaveBeenCalled();
  expect(app.avatarSource(url)).toBe(`${url}?__zoo_avatar=${"a".repeat(64)}`);
  message(app.workers, app.controller);
  expect(alice).toHaveBeenCalledTimes(1);
});

it("refreshes visible images including failed loads on focus, reconnection and the shared interval", async () => {
  const app = await setup();
  stops.push(app.subscribeAvatar(url, vi.fn()));
  app.images.push(
    { ...app.images[0], src: "https://other.example/avatar.png" },
    { ...app.images[0], naturalWidth: 0 },
    {
      ...app.images[0],
      parentElement: {
        getBoundingClientRect: () => ({ top: 900, bottom: 932, left: 0, right: 32 }),
      },
    },
  );
  await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
  expect(app.controller.postMessage).toHaveBeenLastCalledWith({
    type: "REFRESH_AVATARS",
    urls: [url],
  });
  expect(app.controller.postMessage).toHaveBeenCalledTimes(1);
  app.document.visibilityState = "hidden";
  app.window.dispatchEvent(new Event("focus"));
  expect(app.controller.postMessage).toHaveBeenCalledTimes(1);
  app.document.visibilityState = "visible";
  app.navigator.onLine = false;
  app.document.dispatchEvent(new Event("visibilitychange"));
  expect(app.controller.postMessage).toHaveBeenCalledTimes(1);
  app.navigator.onLine = true;
  app.window.dispatchEvent(new Event("online"));
  expect(app.controller.postMessage).toHaveBeenCalledTimes(2);
  stops.splice(0).forEach((stop) => stop());
  await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
  app.window.dispatchEvent(new Event("focus"));
  expect(app.controller.postMessage).toHaveBeenCalledTimes(2);
});

it("keeps ordinary image sources and works before a SW is available", async () => {
  vi.stubGlobal("navigator", {});
  vi.stubGlobal("window", {});
  const app = await import("./avatar-cache");
  const notify = vi.fn();
  stops.push(app.subscribeAvatar(url, notify));
  expect(app.avatarSource(url)).toBe(url);
  expect(app.avatarSource("/icons/zoo.svg")).toBe("/icons/zoo.svg");
  expect(app.avatarSource(undefined)).toBeUndefined();
  expect(notify).not.toHaveBeenCalled();
});
