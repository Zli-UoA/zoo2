import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import type { FeedMessage } from "./messageFormat";
import { ThreadPane, type ThreadPaneProps } from "./ThreadPane";

function message(overrides: Partial<FeedMessage> = {}): FeedMessage {
  return {
    id: "root-1",
    user_id: "user-1",
    content: "root content",
    created_at: "2026-01-02 00:00:00.000",
    parent_id: null,
    user_name: "alice",
    display_name: "Alice",
    ...overrides,
  };
}

function noop() {}

function render(props: Partial<ThreadPaneProps> = {}): string {
  return renderToStaticMarkup(
    createElement(LocaleProvider, {
      children: createElement(ThreadPane, {
        root: message(),
        replies: [],
        isLoading: false,
        isError: false,
        currentUserId: "user-1",
        reactions: new Map(),
        kitchenCatalog: null,
        canReact: false,
        onReaction: noop,
        onPickReaction: noop,
        editingId: null,
        editingText: "",
        onEditingTextChange: noop,
        onStartEdit: noop,
        onCancelEdit: noop,
        onSaveEdit: noop,
        onRequestDelete: noop,
        onSendReply: async () => true,
        onClose: noop,
        ...props,
      }),
    }),
  );
}

describe("ThreadPane", () => {
  it("shows the root message and the reply count", () => {
    const html = render({
      replies: [
        message({
          id: "reply-1",
          user_id: "user-2",
          content: "a reply",
          parent_id: "root-1",
          user_name: "bob",
          display_name: "Bob",
        }),
      ],
    });

    expect(html).toContain("root content");
    expect(html).toContain("a reply");
    expect(html).toContain("1 件の返信");
    expect(html).toContain('placeholder="返信を送信"');
  });

  it("uses a machine-readable datetime for the root timestamp", () => {
    const html = render();
    expect(html).toMatch(/datetime="2026-01-02T00:00:00\.000Z"/i);
    expect(html).not.toMatch(/datetime="2026-01-02 00:00:00\.000"/i);
  });

  it("shows an empty state for a thread without replies", () => {
    const html = render();
    expect(html).toContain("返信はまだありません");
  });

  it("shows the load error state", () => {
    const html = render({ isError: true });
    expect(html).toContain("返信の取得に失敗しました");
  });
});
