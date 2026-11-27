// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vite-plus/test";
import { usePullToRefresh } from "./usePullToRefresh";

function touch(y: number, count = 1) {
  return {
    touches: Array.from({ length: count }, () => ({ clientY: y })),
  } as unknown as import("react").TouchEvent<HTMLElement>;
}

function setup(scrollTop: number, onRefresh: () => Promise<unknown>) {
  return renderHook(() => usePullToRefresh(onRefresh, () => scrollTop));
}

describe("usePullToRefresh", () => {
  it("しきい値を超える引っ張りだけが再取得を呼ぶ", async () => {
    let resolve!: () => void;
    const refresh = vi.fn(
      () =>
        new Promise<void>((done) => {
          resolve = done;
        }),
    );
    const { result } = setup(0, refresh);

    act(() => result.current.onTouchStart(touch(100)));
    act(() => result.current.onTouchMove(touch(220)));
    expect(result.current.pull).toBe(60);
    act(() => result.current.onTouchMove(touch(400)));
    expect(result.current.pull).toBe(100);

    act(() => result.current.onTouchEnd());
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(result.current.pulling).toBe(true);
    await act(async () => {
      resolve();
    });
    expect(result.current.pulling).toBe(false);
    expect(result.current.pull).toBe(0);
  });

  it("しきい値未満では再取得せず、表示を戻す", () => {
    const refresh = vi.fn(async () => {});
    const { result } = setup(0, refresh);
    act(() => result.current.onTouchStart(touch(100)));
    act(() => result.current.onTouchMove(touch(160)));
    expect(result.current.pull).toBe(30);
    act(() => result.current.onTouchEnd());
    expect(refresh).not.toHaveBeenCalled();
    expect(result.current.pull).toBe(0);
  });

  it("先頭以外や複数指の操作は開始しない", () => {
    const refresh = vi.fn(async () => {});
    const scrolled = setup(10, refresh);
    act(() => scrolled.result.current.onTouchStart(touch(100)));
    act(() => scrolled.result.current.onTouchMove(touch(400)));
    expect(scrolled.result.current.pull).toBe(0);

    const { result } = setup(0, refresh);
    act(() => result.current.onTouchStart(touch(100, 2)));
    act(() => result.current.onTouchMove(touch(400)));
    expect(result.current.pull).toBe(0);
  });

  it("移動中にスクロールすると取消し、引っ張り中は追加操作を無視する", async () => {
    let resolve!: () => void;
    const refresh = vi.fn(
      () =>
        new Promise<void>((done) => {
          resolve = done;
        }),
    );
    let scrollTop = 0;
    const { result } = renderHook(() => usePullToRefresh(refresh, () => scrollTop));

    act(() => result.current.onTouchStart(touch(100)));
    act(() => result.current.onTouchMove(touch(220)));
    expect(result.current.pull).toBe(60);
    act(() => result.current.onTouchMove(touch(140, 2)));
    expect(result.current.pull).toBe(0);

    scrollTop = 10;
    act(() => result.current.onTouchStart(touch(100)));
    act(() => result.current.onTouchMove(touch(400)));
    expect(result.current.pull).toBe(0);

    scrollTop = 0;
    act(() => result.current.onTouchStart(touch(100)));
    act(() => result.current.onTouchMove(touch(220)));
    act(() => result.current.onTouchEnd());
    act(() => result.current.onTouchMove(touch(400)));
    expect(result.current.pull).toBe(60);
    expect(refresh).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolve();
    });
  });

  it("touchcancel は再取得せずに表示を戻す", () => {
    const refresh = vi.fn(async () => {});
    const { result } = setup(0, refresh);
    act(() => result.current.onTouchStart(touch(100)));
    act(() => result.current.onTouchMove(touch(220)));
    act(() => result.current.onTouchCancel());
    expect(result.current.pull).toBe(0);
    act(() => result.current.onTouchEnd());
    expect(refresh).not.toHaveBeenCalled();
  });
});
