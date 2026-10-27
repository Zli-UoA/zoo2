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

  it("throttles rapid typing and clears after idle timeout", async () => {
    await flush(() => root.render(createElement(Probe)));
    expect(state.entries[0]?.display_name).toBe("Alice");

    await flush(() => {
      state.setContent("あ");
      state.setContent("あい");
      state.setContent("あいう");
    });

    expect(state.content).toBe("あいう");
    expect(payloads()).toEqual([{ session_id: expect.any(String), content: "あ" }]);

    await flush(() => vi.advanceTimersByTime(200));
    expect(payloads()).toEqual([
      { session_id: expect.any(String), content: "あ" },
      { session_id: expect.any(String), content: "あいう" },
    ]);

    // After 15 seconds of inactivity, draft and server presence are cleared.
    await flush(() => vi.advanceTimersByTime(15000));
    expect(state.content).toBe("");
    expect(payloads().at(-1)?.content).toBe("");
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
    expect(state.content).toBe("");
    expect(payloads().at(-1)?.content).toBe("");

    await flush(() =>
      resolvePoll([
        { user_id: "alice", user_name: "alice", display_name: "Alice", content: "こんにちは" },
      ]),
    );
    expect(state.entries).toEqual([]);
    visibility.mockRestore();
  });

  it("clears on unmount and handles polling errors gracefully", async () => {
    await flush(() => root.render(createElement(Probe)));
    await flush(() => state.setContent("退室前"));
    mocks.get.mockRejectedValue(new Error("offline"));
    await flush(() => vi.advanceTimersByTime(1000));
    expect(state.entries).toEqual([]);

    await flush(() => root.render(null));
    expect(payloads().at(-1)?.content).toBe("");
  });
});
