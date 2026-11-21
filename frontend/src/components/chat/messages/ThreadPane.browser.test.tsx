import "../../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vite-plus/test";
import { page } from "vite-plus/test/browser/context";

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

function paneProps(overrides: Partial<ThreadPaneProps> = {}): ThreadPaneProps {
  return {
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
    ...overrides,
  };
}

/** 呼び出し元のボタンを残したまま ThreadPane を開閉する。 */
function Harness({ onClose }: { onClose?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <LocaleProvider>
      <button onClick={() => setOpen(true)}>スレッドを開く</button>
      {open ? (
        <ThreadPane
          {...paneProps({
            onClose: () => {
              onClose?.();
              setOpen(false);
            },
          })}
        />
      ) : null}
    </LocaleProvider>
  );
}

describe("ThreadPane のダイアログ挙動", () => {
  it("狭幅ではダイアログとして開き、Escape で閉じて呼び出し元へフォーカスを戻す", async () => {
    await page.viewport(390, 844);
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);

    const trigger = screen.getByRole("button", { name: "スレッドを開く" });
    await userEvent.click(trigger);

    const dialog = await screen.findByRole("dialog", { name: "スレッド" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    // 開いた直後のフォーカスはペイン内（Base UI の初期フォーカス）にある。
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));

    // Tab を繰り返しても呼び出し元（背後）へフォーカスが戻らない（フォーカストラップ）。
    for (let index = 0; index < 12; index += 1) {
      await userEvent.tab();
      expect(trigger).not.toHaveFocus();
    }

    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onClose).toHaveBeenCalledOnce();
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it("閉じるボタンでも呼び出し元へフォーカスを戻す", async () => {
    await page.viewport(390, 844);
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);

    const trigger = screen.getByRole("button", { name: "スレッドを開く" });
    await userEvent.click(trigger);
    await screen.findByRole("dialog", { name: "スレッド" });

    await userEvent.click(screen.getByRole("button", { name: "スレッドを閉じる" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onClose).toHaveBeenCalledOnce();
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it("keeps the in-progress reply when the viewport crosses the dialog breakpoint", async () => {
    await page.viewport(390, 844);
    render(<Harness />);
    await userEvent.click(screen.getByRole("button", { name: "スレッドを開く" }));
    const input = await screen.findByRole("textbox", { name: "返信を送信" });
    await userEvent.type(input, "下書きの途中");

    // 縦 → 横 → 縦で 640px をまたいでも、同じ要素ツリーなので下書きが残る。
    await page.viewport(844, 390);
    await waitFor(() =>
      expect(screen.getByRole("complementary", { name: "スレッド" })).toBeVisible(),
    );
    expect(screen.getByRole("textbox", { name: "返信を送信" })).toHaveValue("下書きの途中");

    await page.viewport(390, 844);
    await waitFor(() => expect(screen.getByRole("dialog", { name: "スレッド" })).toBeVisible());
    expect(screen.getByRole("textbox", { name: "返信を送信" })).toHaveValue("下書きの途中");
  });

  it("moves focus back into the dialog when switching from wide to narrow again", async () => {
    await page.viewport(390, 844);
    render(<Harness />);
    const trigger = screen.getByRole("button", { name: "スレッドを開く" });
    await userEvent.click(trigger);
    const dialog = await screen.findByRole("dialog", { name: "スレッド" });

    // 広幅にして背後のトリガーへフォーカスを移してから、狭幅へ戻す。
    await page.viewport(1280, 900);
    await waitFor(() => expect(dialog).toHaveAttribute("role", "complementary"));
    trigger.focus();
    expect(dialog.contains(document.activeElement)).toBe(false);

    await page.viewport(390, 844);
    await waitFor(() => expect(dialog).toHaveAttribute("role", "dialog"));
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
  });

  it("広幅ではインラインのサイドペインのままダイアログにならない", async () => {
    await page.viewport(1280, 900);
    render(<Harness />);

    await userEvent.click(screen.getByRole("button", { name: "スレッドを開く" }));

    expect(await screen.findByRole("complementary", { name: "スレッド" })).toBeVisible();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("moves focus to the reply box once joining makes it available", async () => {
    await page.viewport(1200, 800);
    function JoinHarness() {
      const [membership, setMembership] = useState<"none" | "joined">("none");
      return (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              membership,
              participantCount: 0,
              onJoinThread: () => setMembership("joined"),
            })}
          />
        </LocaleProvider>
      );
    }
    render(<JoinHarness />);
    // The reply box is hidden until the thread is joined.
    expect(screen.queryByRole("textbox", { name: "返信を送信" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "参加する" }));
    const input = await screen.findByRole("textbox", { name: "返信を送信" });
    await waitFor(() => expect(input).toHaveFocus());
  });

  describe("参加者のポップアウト", () => {
    const participants = [
      { id: "u1", user_name: "alice", display_name: "Alice", avatar_url: null },
      { id: "u2", user_name: "bob", display_name: null, avatar_url: null },
    ];
    function ParticipantsHarness({ onClose }: { onClose?: () => void }) {
      return (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              replies: [message({ id: "reply-1", parent_id: "root-1", content: "a reply" })],
              membership: "joined",
              participantCount: 3,
              participants,
              onLeaveThread: noop,
              onClose: onClose ?? noop,
            })}
          />
        </LocaleProvider>
      );
    }

    it("広幅では人数ボタンから開き、ペインの中身を押し下げずに、Escape で一覧だけを閉じる", async () => {
      await page.viewport(1280, 900);
      const onClose = vi.fn();
      render(<ParticipantsHarness onClose={onClose} />);
      const toggle = screen.getByRole("button", { name: "参加者 3人" });
      const reply = screen.getByText("a reply");
      const before = reply.getBoundingClientRect().top;
      expect(toggle).toHaveAttribute("aria-expanded", "false");

      await userEvent.click(toggle);
      const popup = await screen.findByRole("dialog", { name: "参加者 3人" });
      expect(toggle).toHaveAttribute("aria-expanded", "true");
      expect(popup.textContent).toMatch(/Alice/);
      expect(popup.textContent).toMatch(/@alice/);
      expect(popup.textContent).toMatch(/bob/);
      expect(popup.textContent).toMatch(/ほか 1人/);
      // インラインの帯ではなくポップアウト: 返信の位置は変わらず、一覧はペインの外側（body）に重なる。
      expect(reply.getBoundingClientRect().top).toBe(before);
      expect(screen.getByRole("complementary", { name: "スレッド" }).contains(popup)).toBe(false);

      await userEvent.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByRole("dialog", { name: "参加者 3人" })).toBeNull());
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      await waitFor(() => expect(toggle).toHaveFocus());
      expect(onClose).not.toHaveBeenCalled();
    });

    it("狭幅では一覧をダイアログの中へ描画し、Escape でスレッドのペインを閉じずに一覧だけを閉じる", async () => {
      await page.viewport(390, 844);
      const onClose = vi.fn();
      render(<ParticipantsHarness onClose={onClose} />);
      const pane = screen.getByRole("dialog", { name: "スレッド" });
      const toggle = screen.getByRole("button", { name: "参加者 3人" });

      await userEvent.click(toggle);
      const popup = await screen.findByRole("dialog", { name: "参加者 3人" });
      // aria-modal のペインの外に出ると、読み上げ・フォーカス移動の範囲から外れる。
      expect(pane.contains(popup)).toBe(true);
      expect(popup.textContent).toMatch(/Alice/);
      const rect = popup.getBoundingClientRect();
      expect(rect.left).toBeGreaterThanOrEqual(0);
      expect(rect.right).toBeLessThanOrEqual(window.innerWidth);

      await userEvent.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByRole("dialog", { name: "参加者 3人" })).toBeNull());
      expect(screen.getByRole("dialog", { name: "スレッド" })).toBeVisible();
      expect(onClose).not.toHaveBeenCalled();
    });
  });
});
