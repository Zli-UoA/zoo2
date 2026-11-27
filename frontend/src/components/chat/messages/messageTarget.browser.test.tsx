import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import { MESSAGE_TARGET_FLASH_MS, useMessageTargetFlash } from "./messageTarget";

afterEach(() => {
  vi.useRealTimers();
});

describe("useMessageTargetFlash", () => {
  it("highlights only the row that was played, and restarts the run each time", () => {
    const { result } = renderHook(() => useMessageTargetFlash());
    expect(result.current.runFor("m1")).toBeUndefined();

    act(() => result.current.play("m1"));
    expect(result.current.runFor("m1")).toBe(1);
    expect(result.current.runFor("m2")).toBeUndefined();

    // 同じ行をもう一度再生すると、行が最初から再生し直せるよう、回数が増える。
    act(() => result.current.play("m1"));
    expect(result.current.runFor("m1")).toBe(2);

    // 別の行に移ると、前の行の強調は外れる。
    act(() => result.current.play("m2"));
    expect(result.current.runFor("m1")).toBeUndefined();
    expect(result.current.runFor("m2")).toBe(3);
  });

  it("clears itself when the animation ends, so a row mounted later does not replay it", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useMessageTargetFlash());
    act(() => result.current.play("m1"));

    // 再生の途中では消えない。
    act(() => {
      vi.advanceTimersByTime(MESSAGE_TARGET_FLASH_MS - 100);
    });
    expect(result.current.runFor("m1")).toBe(1);

    // CSS のアニメーションが終わる時刻に、そろえて消える。
    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(result.current.runFor("m1")).toBeUndefined();
  });

  it("does not hand an ended run to a row mounted before the clearing timer fires", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(() => useMessageTargetFlash());
    act(() => result.current.play("m1"));
    expect(result.current.runFor("m1")).toBe(1);

    // 時刻だけが進み、状態を消すタイマーはまだ動いていない（遅れた）状態。
    // このあとに描かれる行（一覧から外れて戻った行）へは、終わった強調を渡さない。
    vi.setSystemTime(Date.now() + MESSAGE_TARGET_FLASH_MS + 1);
    rerender();
    expect(result.current.runFor("m1")).toBeUndefined();

    // 再生の途中なら、渡す。
    act(() => result.current.play("m1"));
    expect(result.current.runFor("m1")).toBe(2);
    vi.setSystemTime(Date.now() + MESSAGE_TARGET_FLASH_MS - 100);
    rerender();
    expect(result.current.runFor("m1")).toBe(2);
  });

  it("counts the duration again from the latest play", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useMessageTargetFlash());
    act(() => result.current.play("m1"));
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    act(() => result.current.play("m1"));
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    // 最初の再生から 4 秒たっていても、後の再生はまだ続いている。
    expect(result.current.runFor("m1")).toBe(2);
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(result.current.runFor("m1")).toBeUndefined();
  });
});
