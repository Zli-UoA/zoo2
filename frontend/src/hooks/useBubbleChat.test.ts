// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";
import {
  MAX_BUBBLE_CHAT_LENGTH,
  clampBubbleChatContent,
  useBubbleChat,
  type BubbleChatEntry,
} from "./useBubbleChat";

const mocks = vi.hoisted(() => ({ request: vi.fn(), get: vi.fn(), config: {} }));
vi.mock("../lib/electric/collections", () => ({
  getCollections: () => ({ config: mocks.config }),
}));
vi.mock("../lib/api/client", () => ({
  createConfiguredApiClient: () => ({ request: mocks.request, get: mocks.get }),
}));

let root: Root;
let element: HTMLDivElement;
let state: ReturnType<typeof useBubbleChat>;

function Probe({ enabled = true }: { enabled?: boolean }) {
  state = useBubbleChat(enabled);
  return null;
}

async function flush(action: () => unknown) {
  await act(async () => {
    action();
    await Promise.resolve();
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  mocks.request.mockReset().mockResolvedValue({ ok: true });
  mocks.get.mockReset().mockResolvedValue([
    {
      user_id: "alice",
      user_name: "alice",
      display_name: "Alice",
      content: "こんにちは",
    },
  ]);
  element = document.createElement("div");
  document.body.append(element);
  root = createRoot(element);
});

afterEach(async () => {
  await flush(() => root.unmount());
  element.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const payloads = () => mocks.request.mock.calls.map((call) => call[1].body);

describe("useBubbleChat", () => {
  it("clamps content to MAX_BUBBLE_CHAT_LENGTH code points", () => {
    const longText = "🫧".repeat(MAX_BUBBLE_CHAT_LENGTH + 10);
    expect(Array.from(clampBubbleChatContent(longText)).length).toBe(MAX_BUBBLE_CHAT_LENGTH);
  });

  it("throttles rapid typing, stops sharing after idle timeout, but retains input content", async () => {
    await flush(() => root.render(createElement(Probe)));
    expect(state.entries[0]?.display_name).toBe("Alice");

    await flush(() => {
      state.setContent("あ");
      state.setContent("あい");
      state.setContent("あいう");
    });

    expect(state.content).toBe("あいう");
    expect(state.isSharing).toBe(true);
    expect(payloads()).toEqual([
      {
        session_id: expect.any(String),
        seq: 1,
        content: "あ",
        sent_at: expect.any(Number),
      },
    ]);

    await flush(() => vi.advanceTimersByTime(200));
    expect(payloads()).toEqual([
      {
        session_id: expect.any(String),
        seq: 1,
        content: "あ",
        sent_at: expect.any(Number),
      },
      {
        session_id: expect.any(String),
        seq: 2,
        content: "あいう",
        sent_at: expect.any(Number),
      },
    ]);

    // After 15 seconds of inactivity, sharing stops on the server but the draft input remains.
    await flush(() => vi.advanceTimersByTime(15000));
    expect(state.isSharing).toBe(false);
    expect(state.content).toBe("あいう");
    expect(payloads().at(-1)?.content).toBe("");
    expect(payloads().at(-1)?.seq).toBe(3);

    // Typing again restarts sharing.
    await flush(() => state.setContent("あいうえ"));
    expect(state.isSharing).toBe(true);
    expect(state.content).toBe("あいうえ");
    expect(payloads().at(-1)?.content).toBe("あいうえ");
    expect(payloads().at(-1)?.seq).toBe(4);
  });

  it("coalesces intermediate throttled writes while a request is still in flight", async () => {
    let resolveFirst!: (value: { ok: boolean }) => void;
    mocks.request.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
    );

    await flush(() => root.render(createElement(Probe)));

    // First input starts an in-flight POST immediately.
    await flush(() => state.setContent("1"));
    expect(payloads()).toEqual([
      {
        session_id: expect.any(String),
        seq: 1,
        content: "1",
        sent_at: expect.any(Number),
      },
    ]);

    // Multiple throttled intervals pass while the first POST is still in flight.
    await flush(() => state.setContent("12"));
    await flush(() => vi.advanceTimersByTime(200));
    await flush(() => state.setContent("123"));
    await flush(() => vi.advanceTimersByTime(200));
    await flush(() => state.setContent("1234"));

    // Still only the first request has been dispatched so far.
    expect(payloads()).toEqual([
      {
        session_id: expect.any(String),
        seq: 1,
        content: "1",
        sent_at: expect.any(Number),
      },
    ]);

    // Once the first request resolves, only the latest content ("1234") is sent.
    await flush(() => resolveFirst({ ok: true }));
    expect(payloads()).toEqual([
      {
        session_id: expect.any(String),
        seq: 1,
        content: "1",
        sent_at: expect.any(Number),
      },
      {
        session_id: expect.any(String),
        seq: 2,
        content: "1234",
        sent_at: expect.any(Number),
      },
    ]);
  });

  it("clears immediately when text is emptied or clearContent is called", async () => {
    await flush(() => root.render(createElement(Probe)));
    await flush(() => state.setContent("テスト"));
    expect(payloads().at(-1)?.content).toBe("テスト");

    await flush(() => state.clearContent());
    expect(state.content).toBe("");
    expect(payloads().at(-1)?.content).toBe("");
  });

  it("clears on hidden tab and ignores in-flight poll results after hiding", async () => {
    let resolvePoll!: (entries: BubbleChatEntry[]) => void;
    mocks.get.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolvePoll = done;
        }),
    );
    await flush(() => root.render(createElement(Probe)));
    await flush(() => state.setContent("入力中"));
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    await flush(() => document.dispatchEvent(new Event("visibilitychange")));
    expect(state.isSharing).toBe(false);
    expect(payloads().at(-1)?.content).toBe("");

    await flush(() =>
      resolvePoll([
        { user_id: "alice", user_name: "alice", display_name: "Alice", content: "こんにちは" },
      ]),
    );
    expect(state.entries).toEqual([]);
    visibility.mockRestore();
  });

  it("clears on unmount and retains entries on transient polling errors", async () => {
    await flush(() => root.render(createElement(Probe)));
    await flush(() => state.setContent("退室前"));
    mocks.get.mockRejectedValue(new Error("offline"));
    await flush(() => vi.advanceTimersByTime(1000));
    expect(state.entries[0]?.display_name).toBe("Alice");

    await flush(() => root.render(null));
    expect(payloads().at(-1)?.content).toBe("");
  });

  it("retries write after failure when content is unconfirmed, superseded by newer input", async () => {
    await flush(() => root.render(createElement(Probe)));

    // Both initial request and first retry fail
    mocks.request.mockRejectedValueOnce(new Error("network failure"));
    mocks.request.mockRejectedValueOnce(new Error("second network failure"));
    await flush(() => state.setContent("リトライ確認"));
    expect(payloads()).toEqual([
      {
        session_id: expect.any(String),
        seq: 1,
        content: "リトライ確認",
        sent_at: expect.any(Number),
      },
    ]);

    // Retry timer fires after 1000ms
    await flush(() => vi.advanceTimersByTime(1000));
    expect(payloads()).toEqual([
      {
        session_id: expect.any(String),
        seq: 1,
        content: "リトライ確認",
        sent_at: expect.any(Number),
      },
      {
        session_id: expect.any(String),
        seq: 2,
        content: "リトライ確認",
        sent_at: expect.any(Number),
      },
    ]);

    // Second retry would fire in another 1000ms, but user types new content immediately
    mocks.request.mockResolvedValueOnce({ ok: true });
    await flush(() => state.setContent("最新の入力"));
    await flush(() => vi.advanceTimersByTime(200));
    expect(payloads().at(-1)).toEqual({
      session_id: expect.any(String),
      seq: 3,
      content: "最新の入力",
      sent_at: expect.any(Number),
    });

    // Advance 1000ms to ensure no redundant retry of old content fires
    await flush(() => vi.advanceTimersByTime(1000));
    expect(payloads().length).toBe(3);
  });

  it("retries clearing when previous write was unconfirmed and clear POST fails", async () => {
    await flush(() => root.render(createElement(Probe)));

    // Initial write succeeds or fails unconfirmed
    mocks.request.mockRejectedValueOnce(new Error("network timeout"));
    await flush(() => state.setContent("消去前入力"));
    expect(payloads().at(-1)?.content).toBe("消去前入力");

    // Clear request also fails
    mocks.request.mockRejectedValueOnce(new Error("clear failed"));
    await flush(() => state.clearContent());
    expect(payloads().at(-1)?.content).toBe("");
    expect(payloads().at(-1)?.seq).toBe(2);

    // Because previous content was unconfirmed on server, clear must be retried
    mocks.request.mockResolvedValueOnce({ ok: true });
    await flush(() => vi.advanceTimersByTime(1000));
    expect(payloads().at(-1)?.content).toBe("");
    expect(payloads().at(-1)?.seq).toBe(3);

    // After success, no further retries
    await flush(() => vi.advanceTimersByTime(2000));
    expect(payloads().length).toBe(3);
  });

  it("sends keepalive clear on pagehide even if previous clear completed while newer input is in flight", async () => {
    let resolveClear!: (value: { ok: boolean }) => void;
    let resolveSecondInput!: (value: { ok: boolean }) => void;

    await flush(() => root.render(createElement(Probe)));

    // 1. Initial write "A" succeeds
    await flush(() => state.setContent("A"));
    expect(payloads().at(-1)?.content).toBe("A");

    // 2. Clear request starts and hangs in flight
    mocks.request.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveClear = resolve;
        }),
    );
    await flush(() => state.clearContent());
    expect(payloads().at(-1)?.content).toBe("");
    expect(payloads().at(-1)?.seq).toBe(2);

    // 3. User types "B" while clear is still in flight
    mocks.request.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveSecondInput = resolve;
        }),
    );
    await flush(() => state.setContent("B"));

    // 4. Clear resolves
    await flush(() => resolveClear({ ok: true }));

    // Now B is dispatched and hanging in flight
    expect(payloads().at(-1)?.content).toBe("B");
    expect(payloads().at(-1)?.seq).toBe(3);

    // 5. pagehide occurs while B is in flight
    await flush(() => window.dispatchEvent(new Event("pagehide")));

    // B should be aborted and keepalive clear must be dispatched!
    const lastCall = mocks.request.mock.calls.at(-1);
    expect(lastCall).toBeDefined();
    expect(lastCall?.[1].keepalive).toBe(true);
    expect(lastCall?.[1].body).toEqual({
      session_id: expect.any(String),
      seq: 4,
      content: "",
      sent_at: expect.any(Number),
    });

    resolveSecondInput({ ok: true });
  });

  it("aborts in-flight request and sends keepalive clear on pagehide", async () => {
    let capturedSignal: AbortSignal | undefined;
    mocks.request.mockImplementationOnce((_path, options) => {
      capturedSignal = options?.signal;
      return new Promise(() => {}); // hung in flight
    });

    await flush(() => root.render(createElement(Probe)));
    await flush(() => state.setContent("移動前テキスト"));

    expect(capturedSignal?.aborted).toBe(false);

    await flush(() => window.dispatchEvent(new Event("pagehide")));

    expect(capturedSignal?.aborted).toBe(true);
    const lastCall = mocks.request.mock.calls.at(-1);
    expect(lastCall).toBeDefined();
    expect(lastCall?.[1].keepalive).toBe(true);
    expect(lastCall?.[1].body).toEqual({
      session_id: expect.any(String),
      seq: 2,
      content: "",
      sent_at: expect.any(Number),
    });
  });

  it("aborts in-flight GET on visibility change to hidden, ignores stale results, and maintains single poll loop", async () => {
    let capturedSignal: AbortSignal | undefined;
    let resolveStaleGet!: (data: BubbleChatEntry[]) => void;

    mocks.get.mockImplementationOnce((_path, options) => {
      capturedSignal = options?.signal;
      return new Promise((resolve) => {
        resolveStaleGet = resolve;
      });
    });

    await flush(() => root.render(createElement(Probe)));

    // First poll is in flight
    expect(capturedSignal?.aborted).toBe(false);

    // Tab is hidden
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    await flush(() => document.dispatchEvent(new Event("visibilitychange")));

    // In-flight GET should be aborted immediately
    expect(capturedSignal?.aborted).toBe(true);
    expect(state.entries).toEqual([]);

    // Tab is visible again
    visibility.mockReturnValue("visible");
    mocks.get.mockResolvedValueOnce([
      { user_id: "bob", user_name: "bob", display_name: "Bob", content: "新しい情報" },
    ]);
    await flush(() => document.dispatchEvent(new Event("visibilitychange")));

    // Stale GET finishes now
    resolveStaleGet([
      { user_id: "alice", user_name: "alice", display_name: "Alice", content: "古い情報" },
    ]);
    await flush(() => vi.advanceTimersByTime(50));

    // Stale result should NOT overwrite entries
    expect(state.entries[0]?.content).toBe("新しい情報");

    // Only a single polling loop continues
    const getCallCount = mocks.get.mock.calls.length;
    await flush(() => vi.advanceTimersByTime(1000));
    expect(mocks.get.mock.calls.length).toBe(getCallCount + 1);

    visibility.mockRestore();
  });

  it("clears entries if polling errors persist beyond IDLE_TIMEOUT_MS", async () => {
    await flush(() => root.render(createElement(Probe)));
    expect(state.entries.length).toBe(1);

    // Initial polling error keeps entries
    mocks.get.mockRejectedValue(new Error("offline"));
    await flush(() => vi.advanceTimersByTime(1000));
    expect(state.entries.length).toBe(1);

    // Polling failure continues for 15 seconds
    await flush(() => vi.advanceTimersByTime(14000));
    expect(state.entries).toEqual([]);
  });
});
