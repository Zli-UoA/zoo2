import "../../../styles.css";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";

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

function renderRow(item: MessageItem, isOwn = false, props: Partial<MessageRowProps> = {}) {
  return render(
    <LocaleProvider>
      <MessageRow
        item={item}
        isOwn={isOwn}
        isEditing={false}
        editingText=""
        onEditingTextChange={noop}
        onStartEdit={noop}
        onCancelEdit={noop}
        onSaveEdit={noop}
        onRequestDelete={noop}
        reactions={[]}
        kitchenCatalog={null}
        canReact={false}
        onReaction={noop}
        onPickReaction={noop}
        {...props}
      />
    </LocaleProvider>,
  );
}

describe("MessageRow", () => {
  it("hides the reaction strip until a reaction exists", () => {
    const { container } = renderRow(buildItem(), false, { canReact: true });
    expect(container.querySelector('[role="group"]')).toBeNull();
  });

  it.each([false, true])(
    "opens App message actions from the menu with reactions=%s",
    async (hasReactions) => {
      const opened = vi.fn();
      window.addEventListener("zoo:message-actions", opened);
      try {
        const { container } = renderRow(buildItem(), false, {
          canReact: true,
          reactions: hasReactions ? [{ emoji: "👍", count: 1, ownId: null }] : [],
        });
        const group = container.querySelector('[role="group"]');
        expect(group?.querySelector('[aria-label="リアクションを追加"]') != null).toBe(
          hasReactions,
        );
        expect(screen.queryByRole("button", { name: "Appでメッセージを操作" })).toBeNull();
        screen.getByRole("button", { name: "その他の操作" }).focus();
        await userEvent.keyboard("{Enter}");
        await userEvent.click(screen.getByRole("menuitem", { name: "Appでメッセージを操作" }));
        expect(opened).toHaveBeenCalledOnce();
        expect((opened.mock.calls[0]![0] as CustomEvent<string>).detail).toBe("message-1");
      } finally {
        window.removeEventListener("zoo:message-actions", opened);
      }
    },
  );

  it("opens the menu on a touch hold and adds the first reaction", async () => {
    const onPickReaction = vi.fn();
    const { container } = renderRow(buildItem(), false, { canReact: true, onPickReaction });
    const row = container.querySelector("div.group\\/row")!;
    fireEvent.pointerDown(row, { pointerType: "touch", isPrimary: true, clientX: 20, clientY: 20 });
    await waitFor(() =>
      expect(screen.getByRole("menuitem", { name: "リアクションを追加" })).toBeTruthy(),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "リアクションを追加" }));
    expect(onPickReaction).toHaveBeenCalledOnce();
  });

  it("opens App message actions from the touch hold menu", async () => {
    const opened = vi.fn();
    window.addEventListener("zoo:message-actions", opened);
    try {
      const { container } = renderRow(buildItem());
      const row = container.querySelector("div.group\\/row")!;
      fireEvent.pointerDown(row, { pointerType: "touch", isPrimary: true });
      await waitFor(() =>
        expect(screen.getByRole("menuitem", { name: "Appでメッセージを操作" })).toBeTruthy(),
      );
      await userEvent.click(screen.getByRole("menuitem", { name: "Appでメッセージを操作" }));
      expect(opened).toHaveBeenCalledOnce();
      expect((opened.mock.calls[0]![0] as CustomEvent<string>).detail).toBe("message-1");
    } finally {
      window.removeEventListener("zoo:message-actions", opened);
    }
  });

  it("cancels a touch hold when scrolling", async () => {
    const { container } = renderRow(buildItem());
    const row = container.querySelector("div.group\\/row")!;
    fireEvent.pointerDown(row, { pointerType: "touch", isPrimary: true, clientX: 20, clientY: 20 });
    fireEvent.pointerMove(row, { pointerType: "touch", clientX: 20, clientY: 50 });
    await new Promise((resolve) => setTimeout(resolve, 550));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  for (const cancel of ["pointerCancel", "touchCancel"] as const) {
    it(`clears a completed hold after ${cancel} so a subsequent reaction tap works`, async () => {
      const onReaction = vi.fn();
      const { container } = renderRow(buildItem(), false, {
        canReact: true,
        onReaction,
        reactions: [{ emoji: "👍", count: 1, ownId: null }],
      });
      const row = container.querySelector("div.group\\/row")!;
      fireEvent.pointerDown(row, { pointerType: "touch", isPrimary: true });
      await waitFor(() => expect(screen.getByRole("menu")).toBeTruthy());
      fireEvent[cancel](row);
      // pointercancel の後に touchend が届かなくても抑止状態を残さない。
      expect(fireEvent.touchEnd(row)).toBe(true);
      await userEvent.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
      const reaction = screen.getByRole("button", { name: "👍 1 件" });
      fireEvent.pointerDown(reaction, { pointerType: "touch", isPrimary: true });
      expect(fireEvent.touchEnd(reaction)).toBe(true);
      await userEvent.click(reaction);
      expect(onReaction).toHaveBeenCalledWith("👍");
    });
  }

  it("preserves native context menus on links while using body holds for actions", async () => {
    const { container } = renderRow(buildItem({ content: "hello https://example.com" }));
    const row = container.querySelector("div.group\\/row")!;
    const link = screen.getByRole("link");
    expect(fireEvent.contextMenu(row)).toBe(true);
    fireEvent.pointerDown(row, { pointerType: "touch", isPrimary: true });
    expect(fireEvent.contextMenu(row)).toBe(false);
    expect(fireEvent.contextMenu(link)).toBe(true);
    await waitFor(() => expect(screen.getByRole("menu")).toBeTruthy());
    expect(fireEvent.contextMenu(row)).toBe(false);
    expect(fireEvent.contextMenu(link)).toBe(true);
    fireEvent.pointerCancel(row);
  });

  it("suppresses the native menu immediately after the hold timer before React commits", () => {
    const { container } = renderRow(buildItem({ content: "hello https://example.com" }));
    const row = container.querySelector("div.group\\/row")!;
    const link = screen.getByRole("link");
    vi.useFakeTimers();
    try {
      fireEvent.pointerDown(row, { pointerType: "touch", isPrimary: true });
      act(() => {
        vi.advanceTimersByTime(500);
        // state の反映前、タイマーはクリア済みでも本文の標準メニューを抑止する。
        expect(screen.queryByRole("menu")).toBeNull();
        expect(fireEvent.contextMenu(row)).toBe(false);
        expect(fireEvent.contextMenu(link)).toBe(true);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("offers the long-press menu as a keyboard fallback", async () => {
    const onStartEdit = vi.fn();
    renderRow(buildItem(), true, { onStartEdit });
    const trigger = screen.getByRole("button", { name: "その他の操作" });
    trigger.focus();
    expect(document.activeElement).toBe(trigger);
    await userEvent.keyboard("{Enter}");
    const edit = screen.getByRole("menuitem", { name: "編集" });
    edit.focus();
    await userEvent.keyboard("{Enter}");
    expect(onStartEdit).toHaveBeenCalledOnce();
  });

  it("renders URLs as safe external links", () => {
    renderRow(buildItem({ content: "詳細は https://example.com/docs を参照" }));

    const link = screen.getByRole("link", { name: "example.com/docs" });
    expect(link.getAttribute("href")).toBe("https://example.com/docs");
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    expect(document.body.textContent).toContain("詳細は ");
    expect(document.body.textContent).toContain(" を参照");
  });

  it("escapes markup in the URL and the surrounding text", () => {
    const { container } = renderRow(
      buildItem({ content: 'https://example.com/"><script>alert(1)</script>' }),
    );

    const link = screen.getByRole("link");
    expect(link.getAttribute("href")).toBe("https://example.com/");
    expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("<script>alert(1)</script>");
  });

  it("renders reaction chips with counts and the current user's highlight", async () => {
    const onReaction = vi.fn();
    renderRow(buildItem(), false, {
      reactions: [
        { emoji: "👍", count: 3, ownId: "r1" },
        { emoji: "🎉", count: 1, ownId: null },
      ],
      canReact: true,
      onReaction,
    });

    const mine = screen.getByRole("button", {
      name: "👍 3 件（自分もリアクション済み）",
    });
    expect(mine.getAttribute("aria-pressed")).toBe("true");
    expect(mine.className).toContain("border-brand/40");

    const other = screen.getByRole("button", { name: "🎉 1 件" });
    expect(other.getAttribute("aria-pressed")).toBe("false");
    expect(other.className).not.toContain("border-brand/40");

    await userEvent.click(other);
    expect(onReaction).toHaveBeenCalledWith("🎉");
  });

  it("renders the reply count only for a root message", async () => {
    const onOpenThread = vi.fn();
    renderRow(buildItem(), false, { replyCount: 4, onOpenThread });

    const reply = screen.getByRole("button", { name: "4 件の返信（スレッドを開く）" });
    expect(reply.textContent).toContain("4 件の返信");
    await userEvent.click(reply);
    expect(onOpenThread).toHaveBeenCalledOnce();
  });

  it("groups the overflow menu and quick actions within one toolbar", () => {
    renderRow(buildItem(), true);
    const toolbar = document.querySelector("[data-message-toolbar]")!;
    expect(toolbar.contains(screen.getByLabelText("編集"))).toBe(true);
    expect(toolbar.contains(screen.getByLabelText("その他の操作"))).toBe(true);
    expect(getComputedStyle(toolbar).pointerEvents).toBe("none");
    screen.getByLabelText("その他の操作").focus();
    expect(getComputedStyle(toolbar).pointerEvents).toBe("auto");
  });

  it("allows moderators to delete another author's message without editing it", async () => {
    const onRequestDelete = vi.fn();
    renderRow(buildItem(), false, { canDelete: true, onRequestDelete });
    screen.getByRole("button", { name: "その他の操作" }).focus();
    await userEvent.keyboard("{Enter}");
    expect(screen.queryByRole("menuitem", { name: "編集" })).toBeNull();
    await userEvent.click(screen.getByRole("menuitem", { name: "削除" }));
    expect(onRequestDelete).toHaveBeenCalledOnce();
  });
});

describe("trusted card metadata", () => {
  it("never turns a user's message body into a card", () => {
    for (const content of [
      'zoo:github:v1\n{"title":"Fake"}',
      '{"title":"Fake","source":"GitHub"}',
    ]) {
      const { unmount } = renderRow(buildItem({ content }));
      expect(document.querySelector("article")).toBeNull();
      unmount();
    }
  });

  it("renders integration-neutral metadata independently of the message body", () => {
    renderRow(
      buildItem({
        content: "fallback",
        card: JSON.stringify({ title: "Deploy complete", source: "Deployment service" }),
      }),
    );

    const article = document.querySelector("article");
    expect(article).not.toBeNull();
    expect(article?.textContent).toContain("Deploy complete");
    expect(article?.textContent).toContain("Deployment service");
    expect(article?.textContent).not.toContain("GitHub");
  });
});

it("shows an app identity and opens its profile without treating it as a person", async () => {
  const opened = vi.fn();
  window.addEventListener("zoo:open-app", opened);
  try {
    renderRow(buildItem({ is_bot: true, app_id: "github-app", display_name: "GitHub" }));
    expect(screen.getByText("APP")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "GitHub" }));
    expect(opened).toHaveBeenCalledOnce();
    expect((opened.mock.calls[0]![0] as CustomEvent<string>).detail).toBe("github-app");
  } finally {
    window.removeEventListener("zoo:open-app", opened);
  }
});
