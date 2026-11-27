// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vite-plus/test";
import { usePostEditState } from "./usePostEditState";

describe("usePostEditState", () => {
  it("編集開始で id と本文を入れ、エラーを消す", () => {
    const { result } = renderHook(() => usePostEditState());
    act(() => result.current.setEditError(true));
    expect(result.current.editError).toBe(true);

    act(() => result.current.startEdit("post-1", "元の本文"));
    expect(result.current.editingId).toBe("post-1");
    expect(result.current.editText).toBe("元の本文");
    expect(result.current.editError).toBe(false);
  });

  it("入力・エラー・進行中を独立して更新できる", () => {
    const { result } = renderHook(() => usePostEditState());
    act(() => result.current.setEditText("書き換え"));
    act(() => result.current.setEditError(true));
    act(() => result.current.setEditPending(true));
    expect(result.current.editText).toBe("書き換え");
    expect(result.current.editError).toBe(true);
    expect(result.current.editPending).toBe(true);
    act(() => result.current.setEditPending(false));
    expect(result.current.editPending).toBe(false);
  });

  it("キャンセルは編集中 id だけを消し、入力は残す", () => {
    const { result } = renderHook(() => usePostEditState());
    act(() => result.current.startEdit("post-1", "編集中"));
    act(() => result.current.cancelEdit());
    expect(result.current.editingId).toBeNull();
    expect(result.current.editText).toBe("編集中");
  });

  it("別の投稿を編集すると、前の id を置き換える", () => {
    const { result } = renderHook(() => usePostEditState());
    act(() => result.current.startEdit("post-1", "最初"));
    act(() => result.current.startEdit("post-2", "次"));
    expect(result.current.editingId).toBe("post-2");
    expect(result.current.editText).toBe("次");
  });
});
