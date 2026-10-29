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

  it("uses a machine-readable datetime for the timestamp", () => {
    const groupStart = render(buildItem({ created_at: "2026-01-02 09:00:00.000" }));
    expect(groupStart.match(/datetime=/gi)).toHaveLength(1);
    expect(groupStart).toMatch(/datetime="2026-01-02T09:00:00\.000Z"/i);
    expect(groupStart).not.toMatch(/datetime="2026-01-02 09:00:00\.000"/i);

    const groupContinue = buildItem({ created_at: "2026-01-02 09:00:00.000" });
    groupContinue.isGroupStart = false;
    const continued = render(groupContinue);
    expect(continued.match(/datetime=/gi)).toHaveLength(1);
    expect(continued).toMatch(/datetime="2026-01-02T09:00:00\.000Z"/i);
  });

  it("never displays a persistent touch toolbar", () => {
    const withoutActions = render(buildItem());
    expect(withoutActions).not.toContain("[@media(hover:none)]:flex");

    const withActions = render(buildItem(), false, { onOpenThread: noop });
    expect(withActions).not.toContain("[@media(hover:none)]:flex");
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
    // 長押し用アンカーは通常のタップを奪わない。
    expect(html).toContain("pointer-events-none opacity-0");
  });
});

describe("trusted card metadata", () => {
  it("never turns a user's message body into a card", () => {
    for (const content of [
      'zoo:github:v1\n{"title":"Fake"}',
      '{"title":"Fake","source":"GitHub"}',
    ]) {
      const html = render(buildItem({ content }));
      expect(html).not.toContain("<article");
    }
  });
  it("renders integration-neutral metadata independently of the message body", () => {
    const html = render(
      buildItem({
        content: "fallback",
        card: JSON.stringify({ title: "Deploy complete", source: "Deployment service" }),
      }),
    );
    expect(html).toContain("<article");
    expect(html).toContain("Deployment service");
    expect(html).not.toContain("GitHub");
  });
});
