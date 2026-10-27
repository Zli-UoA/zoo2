import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";
import type { BubbleChatEntry } from "../../hooks/useBubbleChat";
import { createI18n } from "../../lib/i18n";
import { BubbleChatScreen } from "./BubbleChatScreen";

const state = vi.hoisted(() => ({
  entries: [] as BubbleChatEntry[],
  content: "",
  currentUserId: "user-me",
}));

vi.mock("../../hooks/useBubbleChat", () => ({
  useBubbleChat: () => ({
    entries: state.entries,
    content: state.content,
    setContent: vi.fn(),
    clearContent: vi.fn(),
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
  state.entries = [];
  state.content = "";
  state.currentUserId = "user-me";
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
});
