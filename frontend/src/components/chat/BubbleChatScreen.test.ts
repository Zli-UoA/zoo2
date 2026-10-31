// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import type { BubbleChatEntry } from "../../hooks/useBubbleChat";
import { createI18n } from "../../lib/i18n";
import { BubbleChatScreen, shouldClearBubbleChatFromKey } from "./BubbleChatScreen";

const state = vi.hoisted(() => ({
  entries: [] as BubbleChatEntry[],
  content: "",
  isSharing: true,
  currentUserId: "user-me",
  setContent: vi.fn(),
  clearContent: vi.fn(),
}));

vi.mock("../../hooks/useBubbleChat", () => ({
  useBubbleChat: () => ({
    entries: state.entries,
    content: state.content,
    isSharing: state.isSharing,
    setContent: state.setContent,
    clearContent: state.clearContent,
  }),
}));

vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: state.currentUserId, isLoading: false }),
}));

vi.mock("../../hooks/useRealtime", () => ({
  useUsers: () => ({
    data: [
      {
        id: "user-me",
        user_name: "me",
        display_name: "My Name",
        created_at: "2026-10-28T00:00:00Z",
        updated_at: null,
      },
    ],
  }),
}));

function render() {
  return renderToStaticMarkup(
    createElement(I18nextProvider, { i18n: createI18n("ja") }, createElement(BubbleChatScreen)),
  );
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  state.entries = [];
  state.content = "";
  state.isSharing = true;
  state.currentUserId = "user-me";
  state.setContent.mockReset().mockImplementation((val: string) => {
    state.content = val;
  });
  state.clearContent.mockReset().mockImplementation(() => {
    state.content = "";
  });
});

describe("BubbleChatScreen", () => {
  it("renders empty placeholder when no one is typing", () => {
    const html = render();
    expect(html).toContain("現在入力中のユーザーはいません");
    expect(html).not.toContain('data-bubble-side="left"');
    expect(html).not.toContain('data-bubble-side="right"');
  });

  it("renders other users on the left and own active input on the right", () => {
    state.entries = [
      {
        user_id: "user-alice",
        user_name: "alice",
        display_name: "Alice",
        content: "左側のメッセージ",
      },
      {
        user_id: "user-me",
        user_name: "me",
        display_name: "My Name",
        content: "サーバー上の自分の古い文字列",
      },
    ];
    state.content = "右側の自分のメッセージ";

    const html = render();
    expect(html).not.toContain("現在入力中のユーザーはいません");
    expect(html).toContain('data-bubble-side="left"');
    expect(html).toContain("Alice");
    expect(html).toContain("左側のメッセージ");
    expect(html).toContain('data-bubble-side="right"');
    expect(html).toContain("My Name");
    expect(html).toContain("右側の自分のメッセージ");
    expect(html).not.toContain("サーバー上の自分の古い文字列");
  });

  it("hides own bubble and avatar when own input is empty or whitespace", () => {
    state.entries = [
      {
        user_id: "user-alice",
        user_name: "alice",
        display_name: "Alice",
        content: "入力中",
      },
    ];
    state.content = "   ";

    const html = render();
    expect(html).toContain('data-bubble-side="left"');
    expect(html).not.toContain('data-bubble-side="right"');
  });

  it("hides own bubble when sharing is stopped even if input is not empty", () => {
    state.content = "入力中だけど共有停止中";
    state.isSharing = false;

    const html = render();
    expect(html).not.toContain('data-bubble-side="right"');
    // The input textarea retains the user's text.
    expect(html).toContain("入力中だけど共有停止中");
  });

  it("clears input on Cmd+Enter or Ctrl+Enter when not composing", () => {
    expect(
      shouldClearBubbleChatFromKey({
        key: "Enter",
        metaKey: true,
        ctrlKey: false,
        shiftKey: false,
        isComposing: false,
      }),
    ).toBe(true);
    expect(
      shouldClearBubbleChatFromKey({
        key: "Enter",
        metaKey: false,
        ctrlKey: true,
        shiftKey: false,
        isComposing: false,
      }),
    ).toBe(true);
    expect(
      shouldClearBubbleChatFromKey({
        key: "Enter",
        metaKey: false,
        ctrlKey: false,
        shiftKey: false,
        isComposing: false,
      }),
    ).toBe(false);
    expect(
      shouldClearBubbleChatFromKey({
        key: "Enter",
        metaKey: true,
        ctrlKey: false,
        shiftKey: true,
        isComposing: false,
      }),
    ).toBe(false);
    expect(
      shouldClearBubbleChatFromKey({
        key: "Enter",
        metaKey: true,
        ctrlKey: false,
        shiftKey: false,
        isComposing: true,
      }),
    ).toBe(false);
  });

  it("clears textarea immediately and delays clearContent by POP_DURATION_MS with popping animation on Cmd+Enter", async () => {
    vi.useFakeTimers();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    state.content = "入力途中のテキスト";
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(
          createElement(
            I18nextProvider,
            { i18n: createI18n("ja") },
            createElement(BubbleChatScreen),
          ),
        );
      });

      const textarea = container.querySelector("textarea");
      expect(textarea).not.toBeNull();

      // Press Cmd+Enter
      await act(async () => {
        textarea!.dispatchEvent(
          new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true }),
        );
      });

      // Draft input is cleared immediately
      expect(state.setContent).toHaveBeenCalledWith("");
      // clearContent has NOT been called yet (popping animation in progress)
      expect(state.clearContent).not.toHaveBeenCalled();

      // Popping bubble element is present with animate-bubble-pop class
      const poppingBubble = container.querySelector('[data-bubble-side="right"]');
      expect(poppingBubble?.className).toContain("animate-bubble-pop");

      // Advance timers by 3500ms
      await act(async () => {
        vi.advanceTimersByTime(3500);
      });

      // After 3.5s, clearContent is invoked on the server
      expect(state.clearContent).toHaveBeenCalledTimes(1);
    } finally {
      await act(async () => {
        root.unmount();
      });
      container.remove();
      vi.useRealTimers();
      vi.unstubAllGlobals();
    }
  });

  it("renders participants button with count and toggles participants panel", async () => {
    state.entries = [
      {
        user_id: "user-alice",
        user_name: "alice",
        display_name: "Alice",
        content: "入力中テキスト",
      },
    ];
    state.content = "自分のテキスト";

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => {
        root.render(
          createElement(
            I18nextProvider,
            { i18n: createI18n("ja") },
            createElement(BubbleChatScreen),
          ),
        );
      });

      // 2 participants: Alice + Me
      const button = container.querySelector('button[aria-label="参加者"]');
      expect(button).not.toBeNull();
      expect(button?.textContent).toContain("2");

      // Click to open panel
      await act(async () => {
        button!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });

      expect(container.textContent).toContain("参加者 (2)");
      expect(container.textContent).toContain("Alice");
      expect(container.textContent).toContain("My Name");
      expect(container.textContent).toContain("あなた");
    } finally {
      await act(async () => {
        root.unmount();
      });
      container.remove();
    }
  });
});
