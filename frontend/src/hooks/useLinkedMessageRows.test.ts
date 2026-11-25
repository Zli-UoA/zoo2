// @vitest-environment happy-dom
import { act, renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vite-plus/test";

import {
  createStoryCollections,
  storyChannels,
  storyMessages,
  storyUsers,
} from "../../.storybook/fixtures";
import { CollectionsProvider } from "../lib/electric/collections-context";
import type { MessageRow } from "../lib/electric/types";
import { useChannelHasCiphertext, useLinkedMessageRows } from "./useThread";

const channelId = storyMessages[0]!.channel_id;
const messageId = storyMessages[0]!.id;

function wrapper() {
  const bundle = createStoryCollections({
    users: storyUsers,
    channels: storyChannels,
    messages: storyMessages,
  });
  return {
    bundle,
    wrapper: ({ children }: { children: ReactNode }) =>
      createElement(CollectionsProvider, { value: bundle, children }),
  };
}

afterEach(() => vi.restoreAllMocks());

describe("useLinkedMessageRows", () => {
  it("ID で 1 件のメッセージを、投稿者つきで取る", async () => {
    const { wrapper: Wrapper } = wrapper();
    const { result } = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(result.current.data?.length).toBe(1));
    expect(result.current.data?.[0]).toMatchObject({ id: messageId, display_name: "Alice" });
  });

  it("チャンネルまたはメッセージが決まらない間は、何も取得しない", () => {
    const { wrapper: Wrapper, bundle } = wrapper();
    const original = bundle.messages;
    let touched = 0;
    bundle.messages = ((id: string) => {
      touched += 1;
      return original(id);
    }) as typeof bundle.messages;
    const none = renderHook(() => useLinkedMessageRows(null, messageId), { wrapper: Wrapper });
    const noId = renderHook(() => useLinkedMessageRows(channelId, null), { wrapper: Wrapper });
    expect(none.result.current.data).toBeUndefined();
    expect(noId.result.current.data).toBeUndefined();
    expect(touched).toBe(0);
  });

  it("同じメッセージを指す複数の購読は、同じ問い合わせを分け合う", async () => {
    const { wrapper: Wrapper } = wrapper();
    const first = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    const second = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(second.result.current.data?.length).toBe(1));
    expect(second.result.current.collection).toBe(first.result.current.collection);
  });

  it("別のメッセージは、別の問い合わせにする", async () => {
    const { wrapper: Wrapper } = wrapper();
    const other = storyMessages[1]!;
    const first = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    const second = renderHook(() => useLinkedMessageRows(other.channel_id, other.id), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(second.result.current.data?.length).toBe(1));
    expect(second.result.current.collection).not.toBe(first.result.current.collection);
  });

  it("アカウントごとのコレクション群が違えば、問い合わせも分ける", async () => {
    const one = wrapper();
    const two = wrapper();
    const first = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: one.wrapper,
    });
    const second = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: two.wrapper,
    });
    await waitFor(() => expect(second.result.current.data?.length).toBe(1));
    expect(second.result.current.collection).not.toBe(first.result.current.collection);
  });

  it("片付いた問い合わせは、次の購読で作り直す", async () => {
    const { wrapper: Wrapper } = wrapper();
    const first = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(first.result.current.data?.length).toBe(1));
    const collection = first.result.current.collection!;
    first.unmount();
    await collection.cleanup();
    expect(collection.status).toBe("cleaned-up");

    const second = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    await waitFor(() => expect(second.result.current.data?.length).toBe(1));
    expect(second.result.current.collection).not.toBe(collection);
  });

  /** チャンネルの `messages` の同期が、再試行できない失敗（4xx など）で止まった状態にする。 */
  function failSource(bundle: ReturnType<typeof wrapper>["bundle"]) {
    const source = bundle.messages(channelId) as unknown as {
      _lifecycle: { markError(error: Error): void };
    };
    source._lifecycle.markError(new Error("403"));
  }

  it("同期に失敗した問い合わせは、描画のたびに作り直さない", async () => {
    // ライブラリが、失敗した問い合わせごとに出すログ。
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const { wrapper: Wrapper, bundle } = wrapper();
    failSource(bundle);
    let renders = 0;
    const { result, rerender } = renderHook(
      () => {
        renders += 1;
        return useLinkedMessageRows(channelId, messageId);
      },
      { wrapper: Wrapper },
    );
    const first = result.current.collection;
    for (let i = 0; i < 5; i += 1) rerender();
    await act(() => new Promise((resolve) => setTimeout(resolve, 200)));
    // 描き直しは、頼んだ回数（と、状態の通知）だけで、繰り返し続けない。
    expect(renders).toBeLessThan(12);
    expect(logged.mock.calls.length).toBeLessThanOrEqual(1);
    expect(result.current.isError).toBe(true);
    expect(result.current.collection).toBe(first);
  });

  it("失敗した問い合わせを指す別のリンクも、同じ問い合わせを分け合う", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const { wrapper: Wrapper, bundle } = wrapper();
    failSource(bundle);
    const first = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    const second = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expect(second.result.current.collection).toBe(first.result.current.collection);
    expect(logged.mock.calls.length).toBeLessThanOrEqual(1);
  });

  it("失敗した問い合わせも、購読者がいなくなって片付いたら、次の購読で作り直して取り直す", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const { wrapper: Wrapper, bundle } = wrapper();
    const source = bundle.messages(channelId);
    failSource(bundle);
    const first = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    const failed = first.result.current.collection!;
    expect(failed.status).toBe("error");
    first.unmount();
    // 片付け（購読者がいなくなったあとの GC）。ソースのコレクションも、片付くと取り直せる。
    await failed.cleanup();
    await source.cleanup();

    const second = renderHook(() => useLinkedMessageRows(channelId, messageId), {
      wrapper: Wrapper,
    });
    expect(second.result.current.collection).not.toBe(failed);
    await waitFor(() => expect(second.result.current.data?.length).toBe(1));
  });
});

describe("useChannelHasCiphertext", () => {
  const cipherRow: MessageRow = {
    id: "message-cipher",
    channel_id: channelId,
    user_id: "user-alice",
    content: "",
    created_at: "2026-01-02 09:30:00.000",
    updated_at: null,
    deleted_at: null,
    parent_id: null,
    ciphertext: {
      algorithm: "m.megolm.v1.aes-sha2",
      sender_key: "key",
      device_id: "device",
      session_id: "session",
      ciphertext: "AwgA",
    },
  };

  it("暗号文のある行が読み込まれていなければ false", () => {
    const { wrapper: Wrapper } = wrapper();
    const { result } = renderHook(() => useChannelHasCiphertext(channelId), { wrapper: Wrapper });
    expect(result.current).toBe(false);
  });

  it("暗号文のある行が読み込まれていれば、対象の行でなくても true", () => {
    const bundle = createStoryCollections({
      users: storyUsers,
      channels: storyChannels,
      messages: [...storyMessages, cipherRow],
    });
    const { result } = renderHook(() => useChannelHasCiphertext(channelId), {
      wrapper: ({ children }: { children: ReactNode }) =>
        createElement(CollectionsProvider, { value: bundle, children }),
    });
    expect(result.current).toBe(true);
  });

  it("暗号文のある行があとから読み込まれたら、true に変わる", async () => {
    const { wrapper: Wrapper, bundle } = wrapper();
    const { result } = renderHook(() => useChannelHasCiphertext(channelId), { wrapper: Wrapper });
    expect(result.current).toBe(false);
    await act(async () => {
      bundle.messages(channelId).insert(cipherRow);
    });
    await waitFor(() => expect(result.current).toBe(true));
  });

  it("別のチャンネルの暗号文は見ない", () => {
    const other = storyChannels.find((channel) => channel.id !== channelId)!;
    const bundle = createStoryCollections({
      users: storyUsers,
      channels: storyChannels,
      messages: [...storyMessages, { ...cipherRow, channel_id: other.id }],
    });
    const { result } = renderHook(() => useChannelHasCiphertext(channelId), {
      wrapper: ({ children }: { children: ReactNode }) =>
        createElement(CollectionsProvider, { value: bundle, children }),
    });
    expect(result.current).toBe(false);
  });

  it("チャンネルが決まらない間は、コレクションに触れず false", () => {
    const { wrapper: Wrapper, bundle } = wrapper();
    const original = bundle.messages;
    let touched = 0;
    bundle.messages = ((id: string) => {
      touched += 1;
      return original(id);
    }) as typeof bundle.messages;
    const { result } = renderHook(() => useChannelHasCiphertext(null), { wrapper: Wrapper });
    expect(result.current).toBe(false);
    expect(touched).toBe(0);
  });
});
