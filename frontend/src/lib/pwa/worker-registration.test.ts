import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";

const origin = "https://zoo.example";
const scriptURL = `${origin}/sw.js?v=${"00".repeat(32)}`;

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers();
  vi.stubGlobal("window", { isSecureContext: true, location: { origin } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response("shell")),
  );
  vi.spyOn(crypto.subtle, "digest").mockResolvedValue(new Uint8Array(32).buffer);
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function setup(controlled: boolean, updating: boolean) {
  const active = Object.assign(new EventTarget(), { state: "activated", scriptURL });
  const installing = updating
    ? Object.assign(new EventTarget(), { state: "installing", scriptURL })
    : null;
  const registration = { active, installing, waiting: null, update: vi.fn(async () => {}) };
  const container = Object.assign(new EventTarget(), {
    controller: controlled ? active : null,
    getRegistration: vi.fn(async () => registration),
  });
  vi.stubGlobal("navigator", { serviceWorker: container });
  return { container, installing };
}

it.each([false, true])(
  "does not wait for a settled active SW (controlled=%s)",
  async (controlled) => {
    setup(controlled, false);
    const { prepareOfflineWorker } = await import("./worker-registration");
    let settled = false;
    const preparing = prepareOfflineWorker().then(() => {
      settled = true;
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(settled).toBe(true);
    await preparing;
  },
);

it.each([false, true])(
  "waits for an updating SW to take control (controlled=%s)",
  async (controlled) => {
    const { container, installing } = setup(controlled, true);
    const { prepareOfflineWorker } = await import("./worker-registration");
    let settled = false;
    const preparing = prepareOfflineWorker().then(() => {
      settled = true;
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(settled).toBe(false);
    installing!.state = "activated";
    container.controller = installing;
    container.dispatchEvent(new Event("controllerchange"));
    await preparing;
    expect(settled).toBe(true);
  },
);
