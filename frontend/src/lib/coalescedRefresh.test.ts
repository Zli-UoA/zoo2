import { afterEach, expect, it, vi } from "vite-plus/test";
import { coalescedRefresh } from "./coalescedRefresh";

afterEach(() => vi.useRealTimers());
it("loads immediately and coalesces busy live notifications without starving refresh", () => {
  vi.useFakeTimers();
  const refresh = vi.fn();
  const scheduler = coalescedRefresh(refresh);
  scheduler.request();
  expect(refresh).toHaveBeenCalledTimes(1);
  for (let second = 0; second < 4; second++) {
    vi.advanceTimersByTime(1000);
    scheduler.request();
  }
  expect(refresh).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(1000);
  expect(refresh).toHaveBeenCalledTimes(2);
  vi.advanceTimersByTime(20000);
  expect(refresh).toHaveBeenCalledTimes(2);
  scheduler.request();
  expect(refresh).toHaveBeenCalledTimes(3);
  scheduler.dispose();
});
it("cancels pending work on channel change or unmount", () => {
  vi.useFakeTimers();
  const refresh = vi.fn();
  const scheduler = coalescedRefresh(refresh);
  scheduler.request();
  scheduler.request();
  scheduler.dispose();
  scheduler.request();
  vi.advanceTimersByTime(5000);
  expect(refresh).toHaveBeenCalledTimes(1);
});
it("waits for slow requests and performs one trailing refresh for a burst", async () => {
  vi.useFakeTimers();
  let finish!: () => void;
  const refresh = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const scheduler = coalescedRefresh(refresh);
  scheduler.request();
  for (let event = 0; event < 10; event++) {
    await vi.advanceTimersByTimeAsync(1000);
    scheduler.request();
  }
  expect(refresh).toHaveBeenCalledTimes(1);
  finish();
  await vi.advanceTimersByTimeAsync(0);
  expect(refresh).toHaveBeenCalledTimes(2);
  scheduler.request();
  scheduler.dispose();
  finish();
  await vi.advanceTimersByTimeAsync(10000);
  expect(refresh).toHaveBeenCalledTimes(2);
});
it("can refresh again after a rejected request", async () => {
  vi.useFakeTimers();
  const refresh = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(undefined);
  const scheduler = coalescedRefresh(refresh);
  scheduler.request();
  scheduler.request();
  await vi.advanceTimersByTimeAsync(5000);
  expect(refresh).toHaveBeenCalledTimes(2);
  scheduler.dispose();
});
