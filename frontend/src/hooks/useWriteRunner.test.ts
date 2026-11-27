// @vitest-environment happy-dom
import type { Transaction } from "@tanstack/react-db";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vite-plus/test";

import { createI18n } from "../lib/i18n";
import { localizeApiError } from "../lib/i18n/apiError";
import { useWriteRunner } from "./useWriteRunner";

function transaction(result: Promise<void>): Transaction {
  return { when: () => result } as unknown as Transaction;
}

describe("useWriteRunner", () => {
  it("失敗原因を生のまま残し、ロケールの文言へ変換できるようにする", async () => {
    const cause = new TypeError("Failed to fetch");
    const { result } = renderHook(() => useWriteRunner());

    await act(async () => {
      await result.current.run(() => transaction(Promise.reject(cause)));
    });

    expect(result.current.error).toBe("Failed to fetch");
    expect(result.current.errorCause).toBe(cause);
    // トーストはこの生の原因を共通の変換へ通して現在の言語で表示する。
    expect(localizeApiError(result.current.errorCause, createI18n("ja").t)).toBe(
      "サーバーに接続できませんでした。接続を確認してもう一度お試しください。",
    );
  });

  it("setError で任意の文言を入れると生の原因は消える", async () => {
    const { result } = renderHook(() => useWriteRunner());
    await act(async () => {
      await result.current.run(() => transaction(Promise.reject(new TypeError("Failed to fetch"))));
    });
    expect(result.current.errorCause).toBeInstanceOf(TypeError);

    act(() => result.current.setError("独自のエラー"));

    expect(result.current.error).toBe("独自のエラー");
    expect(result.current.errorCause).toBeNull();
  });

  it("成功した書き込みや再実行では古い原因を残さない", async () => {
    const { result } = renderHook(() => useWriteRunner());
    await act(async () => {
      await result.current.run(() => transaction(Promise.reject(new Error("boom"))));
    });
    expect(result.current.errorCause).toBeInstanceOf(Error);

    await act(async () => {
      await result.current.run(() => transaction(Promise.resolve()));
    });
    expect(result.current.error).toBeNull();
    expect(result.current.errorCause).toBeNull();
  });
});
