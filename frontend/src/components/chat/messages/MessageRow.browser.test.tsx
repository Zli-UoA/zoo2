import "../../../styles.css";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import { describe, expect, it, vi } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";

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
  const element = (overrides: Partial<MessageRowProps>) => (
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
        {...overrides}
      />
    </LocaleProvider>
  );
  const result = render(element(props));
  return {
    ...result,
    update: (next: Partial<MessageRowProps>) => result.rerender(element({ ...props, ...next })),
  };
}

describe("MessageRow", () => {
  it("closes the list when its last reaction disappears and stays closed if it returns", async () => {
    await page.viewport(390, 844);
    const reaction = {
      emoji: "👍",
      count: 1,
      ownId: null,
      userIds: ["u1"],
      userNames: { u1: "Alice" },
    };
    const view = renderRow(buildItem(), false, { reactions: [reaction] });
    await userEvent.click(
      screen.getByRole("button", { name: "👍 にリアクションしたユーザーを表示" }),
    );
    expect(screen.getByRole("dialog")).toBeTruthy();
    view.update({ reactions: [] });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    view.update({ reactions: [reaction] });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("shows an unresolved users message only once", async () => {
    await page.viewport(390, 844);
    renderRow(buildItem(), false, {
      reactions: [{ emoji: "👍", count: 2, ownId: null, userIds: ["u1", "u2"], userNames: {} }],
    });
    await userEvent.click(
      screen.getByRole("button", { name: "👍 にリアクションしたユーザーを表示" }),
    );
    expect(screen.getAllByText("ユーザーを表示できません")).toHaveLength(1);
  });
  it("shows the loading message once alongside resolved users", async () => {
    await page.viewport(390, 844);
    renderRow(buildItem(), false, {
      reactions: [
        {
          emoji: "👍",
          count: 3,
          ownId: null,
          userIds: ["u1", "u2", "u3"],
          userNames: { u1: "Alice" },
          usersLoading: true,
        },
      ],
    });
    await userEvent.click(
      screen.getByRole("button", { name: "👍 にリアクションしたユーザーを表示" }),
    );
    const dialog = within(screen.getByRole("dialog"));
    expect(dialog.getByText("Alice")).toBeTruthy();
    expect(dialog.getAllByText("ユーザーを読み込み中")).toHaveLength(1);
  });
  it("allows keyboard users to open the full reaction list on desktop", async () => {
    await page.viewport(1024, 768);
    renderRow(buildItem(), false, {
      canReact: true,
      reactions: [
        { emoji: "👍", count: 1, ownId: null, userIds: ["u1"], userNames: { u1: "Alice" } },
      ],
    });
    const chip = screen.getByRole("button", { name: "👍 1 件" });
    chip.focus();
    await userEvent.keyboard("{Tab}");
    const listButton = screen.getByRole("button", { name: "👍 にリアクションしたユーザーを表示" });
    expect(document.activeElement).toBe(listButton);
    await userEvent.keyboard("{Enter}");
    expect(screen.getByRole("dialog").textContent).toContain("Alice");
    await page.viewport(390, 844);
  });
  it("shows reaction users on hover without toggling the reaction", async () => {
    await page.viewport(1024, 768);
    const onReaction = vi.fn();
    renderRow(buildItem(), false, {
      canReact: true,
      onReaction,
      reactions: [
        {
          emoji: "👍",
          count: 2,
          ownId: null,
          userIds: ["u1", "u2"],
          userNames: { u1: "Alice", u2: "Bob" },
        },
      ],
    });
    await userEvent.hover(screen.getByRole("button", { name: "👍 2 件" }));
    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip.textContent).toContain("Alice");
    expect(tooltip.textContent).toContain("Bob");
    expect(onReaction).not.toHaveBeenCalled();
    await userEvent.unhover(screen.getByRole("button", { name: "👍 2 件" }));
    screen.getByRole("button", { name: "👍 2 件" }).focus();
    expect((await screen.findByRole("tooltip")).textContent).toContain("Alice");
    await page.viewport(390, 844);
  });
  it("shows reaction users without toggling the reaction", async () => {
    const onReaction = vi.fn();
    renderRow(buildItem(), false, {
      canReact: true,
      onReaction,
      reactions: [
        {
          emoji: "👍",
          count: 2,
          ownId: null,
          userIds: ["u1", "u2"],
          userNames: { u1: "Alice", u2: "Bob" },
        },
      ],
    });
    await userEvent.click(
      screen.getByRole("button", { name: "👍 にリアクションしたユーザーを表示" }),
    );
    expect(screen.getByRole("dialog").textContent).toContain("Alice");
    expect(screen.getByRole("dialog").textContent).toContain("Bob");
    expect(onReaction).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await userEvent.click(screen.getByRole("button", { name: "👍 2 件" }));
    expect(onReaction).toHaveBeenCalledWith("👍");
  });

  it("shows unavailable users when their information cannot be resolved", async () => {
    renderRow(buildItem(), false, {
      reactions: [{ emoji: "👍", count: 1, ownId: null, userIds: ["missing"], userNames: {} }],
    });
    await userEvent.click(
      screen.getByRole("button", { name: "👍 にリアクションしたユーザーを表示" }),
    );
    expect(screen.getByRole("dialog").textContent).toContain("ユーザーを表示できません");
  });
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

describe("copy original message", () => {
  const raw = "  **太字** と [リンク](https://example.com/path?a=1)\n\n:smile: $[spin 原文]  ";
  it.each([null, "thread-root"])(
    "copies unformatted content from a read-only message with parent=%s",
    async (parent_id) => {
      const copy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
      const success = vi.spyOn(toast, "success").mockReturnValue("copy-success");
      try {
        renderRow(buildItem({ content: raw, parent_id }));
        screen.getByRole("button", { name: "その他の操作" }).focus();
        await userEvent.keyboard("{Enter}");
        await userEvent.click(screen.getByRole("menuitem", { name: "原文をコピー" }));
        await waitFor(() => expect(copy).toHaveBeenCalledWith(raw));
        expect(copy).toHaveBeenCalledOnce();
        await waitFor(() => expect(success).toHaveBeenCalledWith("原文をコピーしました"));
      } finally {
        copy.mockRestore();
        success.mockRestore();
      }
    },
  );
  it("reports a denied clipboard write without reporting success", async () => {
    const copy = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockRejectedValue(new DOMException("Denied", "NotAllowedError"));
    const error = vi.spyOn(toast, "error").mockReturnValue("copy-error");
    const success = vi.spyOn(toast, "success").mockReturnValue("copy-success");
    try {
      renderRow(buildItem({ content: raw }));
      screen.getByRole("button", { name: "その他の操作" }).focus();
      await userEvent.keyboard("{Enter}");
      await userEvent.click(screen.getByRole("menuitem", { name: "原文をコピー" }));
      await waitFor(() => expect(error).toHaveBeenCalledWith("原文をコピーできませんでした"));
      expect(success).not.toHaveBeenCalled();
    } finally {
      copy.mockRestore();
      error.mockRestore();
      success.mockRestore();
    }
  });
});

it.each([null, "thread-root"])(
  "places an edited marker after the final inline link (parent=%s)",
  async (parent_id) => {
    renderRow(
      buildItem({
        content: "Updated [link](https://example.com)",
        parent_id,
        updated_at: "2026-01-02 00:10:00.000",
      }),
    );
    const marker = screen.getByRole("img", { name: "編集済み" });
    const link = screen.getByRole("link", { name: "link" });
    expect(marker.getBoundingClientRect().left).toBeGreaterThanOrEqual(
      link.getBoundingClientRect().right,
    );
    expect(
      Math.abs(marker.getBoundingClientRect().top - link.getBoundingClientRect().top),
    ).toBeLessThan(10);
    await userEvent.hover(marker);
    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip.textContent).toContain("編集済み");
    expect(tooltip.querySelector("time")).toHaveAttribute("datetime", "2026-01-02T00:10:00.000Z");
  },
);

it("shows the last edit time when the marker receives keyboard focus", async () => {
  renderRow(
    buildItem({
      content: "Updated [link](https://example.com)",
      updated_at: "2026-01-02 00:10:00.000",
    }),
  );
  screen.getByRole("link", { name: "link" }).focus();
  await userEvent.keyboard("{Tab}");
  expect(screen.getByRole("img", { name: "編集済み" })).toHaveFocus();
  expect((await screen.findByRole("tooltip")).textContent).toContain("編集済み");
});
