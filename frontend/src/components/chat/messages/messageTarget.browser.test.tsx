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

  it("clears itself after the animation has ended, so a row mounted later does not replay it", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useMessageTargetFlash());
    act(() => result.current.play("m1"));

    // 再生の途中では消えない。
    act(() => {
      vi.advanceTimersByTime(MESSAGE_TARGET_FLASH_MS);
    });
    expect(result.current.runFor("m1")).toBe(1);

    // 終わったら消える。
    act(() => {
      vi.advanceTimersByTime(300);
    });
    expect(result.current.runFor("m1")).toBeUndefined();
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
