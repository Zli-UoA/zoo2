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
