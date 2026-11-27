// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import { useLoadingTimeout } from "./useLoadingTimeout";

const advance = (ms: number) => {
  void vi.advanceTimersByTime(ms);
};

describe("useLoadingTimeout", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("非アクティブのときは、時間が経っても true にならない", () => {
    const { result } = renderHook(() => useLoadingTimeout(false, 100));
    act(() => advance(500));
    expect(result.current).toBe(false);
  });

  it("指定した時間が経過したときだけ true を返す", () => {
    const { result } = renderHook(() => useLoadingTimeout(true, 100));
    act(() => advance(99));
    expect(result.current).toBe(false);
    act(() => advance(1));
    expect(result.current).toBe(true);
  });

  it("active が false に戻ると false に戻す", () => {
    const { result, rerender } = renderHook(({ active }) => useLoadingTimeout(active, 100), {
      initialProps: { active: true },
    });
    act(() => advance(100));
    expect(result.current).toBe(true);
    rerender({ active: false });
    expect(result.current).toBe(false);
  });

  it("再アクティブ化するとタイマーを掛け直し、前回の予約は残さない", () => {
    const { result, rerender } = renderHook(
      ({ active, timeoutMs }) => useLoadingTimeout(active, timeoutMs),
      { initialProps: { active: false, timeoutMs: 200 } },
    );
    rerender({ active: true, timeoutMs: 200 });
    act(() => advance(100));
    rerender({ active: false, timeoutMs: 200 });
    act(() => advance(150));
    expect(result.current).toBe(false);
    rerender({ active: true, timeoutMs: 200 });
    act(() => advance(100));
    expect(result.current).toBe(false);
    act(() => advance(100));
    expect(result.current).toBe(true);
  });

  it("アンマウント後はタイマーを片付ける", () => {
    const clearTimeout = vi.spyOn(globalThis, "clearTimeout");
    const { unmount } = renderHook(() => useLoadingTimeout(true, 100));
    unmount();
    expect(clearTimeout).toHaveBeenCalled();
    clearTimeout.mockRestore();
  });
});
