import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";

import { LocaleProvider } from "../../i18n/LocaleProvider";
import { MessageRow, type MessageRowProps } from "./MessageRow";
import type { MessageItem } from "./messageGrouping";

function buildItem(overrides: Partial<MessageItem["message"]> = {}): MessageItem {
  return {
    message: {
      id: "message-1",
      user_id: "user-1",
      content: "hello",
      created_at: "2026-01-02 00:00:00.000",
      parent_id: null,
      user_name: "alice",
      display_name: "Alice",
      ...overrides,
    },
    isGroupStart: true,
    isGroupEnd: true,
    showDateSeparator: false,
  };
}

function noop() {}

function render(item: MessageItem, isOwn = false, props: Partial<MessageRowProps> = {}): string {
  return renderToStaticMarkup(
    createElement(LocaleProvider, {
      children: createElement(MessageRow, {
        item,
        isOwn,
        isEditing: false,
        editingText: "",
        onEditingTextChange: noop,
        onStartEdit: noop,
        onCancelEdit: noop,
        onSaveEdit: noop,
        onRequestDelete: noop,
        reactions: [],
        kitchenCatalog: null,
        canReact: false,
        onReaction: noop,
        onPickReaction: noop,
        ...props,
      }),
    }),
  );
}

describe("MessageRow", () => {
  it("renders URLs as safe external links", () => {
    const html = render(buildItem({ content: "詳細は https://example.com/docs を参照" }));
    expect(html).toContain('href="https://example.com/docs"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain("詳細は ");
    expect(html).toContain(" を参照");
  });

  it("escapes markup in the URL and the surrounding text", () => {
    const html = render(buildItem({ content: 'https://example.com/"><script>alert(1)</script>' }));
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
    expect(html).toContain('href="https://example.com/"');
  });

  it("renders reaction chips with counts and the current user's highlight", () => {
    const html = render(buildItem(), false, {
      reactions: [
        { emoji: "👍", count: 3, ownId: "r1" },
        { emoji: "🎉", count: 1, ownId: null },
      ],
      canReact: true,
    });

    expect(html).toContain("👍");
    expect(html).toContain(">3<");
    expect(html).toContain("🎉");
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('aria-pressed="false"');
    expect(html).toContain("border-brand/40");
  });

  it("renders the reply count only for a root message", () => {
    const html = render(buildItem(), false, {
      replyCount: 4,
      onOpenThread: noop,
    });
    expect(html).toContain("4 件の返信");
    expect(html).toContain('aria-label="4 件の返信（スレッドを開く）"');

    const withoutThread = render(buildItem(), false, { replyCount: 0 });
    expect(withoutThread).not.toContain("件の返信");
  });

  it("keeps the hover actions from stealing clicks while invisible", () => {
    const html = render(buildItem(), true);
    // 外枠ごとクリックを通し、表示（ホバー / フォーカス内）のときだけ操作群を戻す。
    expect(html).toContain("pointer-events-none absolute right-1.5 top-0 flex items-center");
    expect(html).toContain("pointer-events-none hidden items-center gap-0.5");
    expect(html).toContain("group-hover/row:pointer-events-auto");
    expect(html).toContain("group-focus-within/row:pointer-events-auto");
    expect(html).toContain("group-hover/row:opacity-100");
    expect(html).toContain("group-focus-within/row:opacity-100");
    // タッチ用の「⋯」は外枠の指定を打ち消して押せるままにする。
    expect(html).toContain("pointer-events-auto border bg-popover shadow-sm");
  });
});
