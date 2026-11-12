// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vite-plus/test";
import { visiblePoll } from "./visiblePoll";
let polling: ReturnType<typeof visiblePoll> | undefined;
beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
});
afterEach(() => {
  polling?.dispose();
  vi.restoreAllMocks();
  vi.useRealTimers();
});
it("backs off idle polling, resumes immediately and returns to the active interval", async () => {
  const load = vi.fn().mockResolvedValue(false);
  polling = visiblePoll(load, { interval: 1000, maxInterval: 4000 });
  await vi.advanceTimersByTimeAsync(1999);
  expect(load).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(load).toHaveBeenCalledTimes(2);
  await vi.advanceTimersByTimeAsync(4000);
  expect(load).toHaveBeenCalledTimes(3);
  load.mockResolvedValue(true);
  polling.refresh();
  await vi.advanceTimersByTimeAsync(1000);
  expect(load).toHaveBeenCalledTimes(5);
});
it("never overlaps slow requests and coalesces explicit refreshes", async () => {
  let finish!: (active: boolean) => void;
  const load = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        finish = resolve;
      }),
  );
  polling = visiblePoll(load, { interval: 1000, maxInterval: 4000 });
  for (let event = 0; event < 20; event++) polling.refresh();
  await vi.advanceTimersByTimeAsync(5000);
  expect(load).toHaveBeenCalledTimes(1);
  finish(false);
  await vi.advanceTimersByTimeAsync(0);
  expect(load).toHaveBeenCalledTimes(2);
  polling.dispose();
  finish(false);
  await vi.advanceTimersByTimeAsync(20000);
  expect(load).toHaveBeenCalledTimes(2);
});
it("aborts on hide, pauses timers, and fetches immediately on return", async () => {
  let signal!: AbortSignal;
  const load = vi.fn((value: AbortSignal) => {
    signal = value;
    return new Promise<boolean>((_resolve, reject) =>
      value.addEventListener("abort", () => reject(value.reason), { once: true }),
    );
  });
  polling = visiblePoll(load, { interval: 1000 });
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  document.dispatchEvent(new Event("visibilitychange"));
  expect(signal.aborted).toBe(true);
  await vi.advanceTimersByTimeAsync(30000);
  expect(load).toHaveBeenCalledTimes(1);
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  document.dispatchEvent(new Event("visibilitychange"));
  expect(load).toHaveBeenCalledTimes(2);
});
