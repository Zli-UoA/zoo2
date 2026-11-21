import "../../../styles.css";
import { render, screen, waitFor, within } from "@testing-library/react";
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

  // 多くのテストを同時に流す、負荷の高い環境でも安定するよう、ダイアログの開閉・フォーカス復帰を待つ時間を長めにする（既定は1秒）。
  const SLOW = { timeout: 5000 };
  describe("メンバーのダイアログ", () => {
    const participants = [
      { id: "user-1", user_name: "alice", display_name: "Alice", avatar_url: null },
      { id: "user-2", user_name: "bob", display_name: null, avatar_url: null },
      { id: "user-3", user_name: "carol", display_name: "Carol", avatar_url: null },
    ];
    function MembersHarness({
      onClose,
      onLeaveThread,
    }: {
      onClose?: () => void;
      onLeaveThread?: () => void;
    }) {
      return (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              // スレ主（親メッセージの投稿者）は bob、自分は alice (`currentUserId` は user-1)。
              root: message({ user_id: "user-2", user_name: "bob", display_name: null }),
              replies: [message({ id: "reply-1", parent_id: "root-1", content: "a reply" })],
              membership: "joined",
              participantCount: 53,
              participants,
              onLeaveThread: onLeaveThread ?? noop,
              onClose: onClose ?? noop,
            })}
          />
        </LocaleProvider>
      );
    }

    it("画面の中央に出るダイアログに、スレ主・あなたの印、検索窓、退出ボタンを出す", async () => {
      await page.viewport(1280, 900);
      const onClose = vi.fn();
      const onLeaveThread = vi.fn();
      render(<MembersHarness onClose={onClose} onLeaveThread={onLeaveThread} />);
      const toggle = screen.getByRole("button", { name: "参加者 53人" });
      const reply = screen.getByText("a reply");
      const before = reply.getBoundingClientRect().top;

      await userEvent.click(toggle);
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      // ほかのポップアウトと同じ、画面の中央のダイアログ。ペインの中身は動かない。
      const rect = dialog.getBoundingClientRect();
      expect(Math.abs(rect.left + rect.width / 2 - window.innerWidth / 2)).toBeLessThan(2);
      expect(Math.abs(rect.top + rect.height / 2 - window.innerHeight / 2)).toBeLessThan(2);
      expect(reply.getBoundingClientRect().top).toBe(before);

      // 並びは、スレ主 (bob) が先頭、残りは名前順。印は、スレ主と自分。
      const rows = within(within(dialog).getByRole("list", { name: "参加者 53人" })).getAllByRole(
        "listitem",
      );
      expect(rows.map((row) => row.textContent)).toEqual([
        expect.stringMatching(/^.*bob.*スレ主$/),
        expect.stringMatching(/^.*Alice.*あなた$/),
        expect.stringMatching(/Carol/),
      ]);
      expect(within(dialog).getByText("ほか 50人")).toBeTruthy();

      // 名前で絞り込む。
      const search = within(dialog).getByRole("searchbox", { name: "名前で検索" });
      await userEvent.type(search, "car");
      expect(
        within(within(dialog).getByRole("list"))
          .getAllByRole("listitem")
          .map((row) => row.textContent),
      ).toEqual([expect.stringMatching(/Carol/)]);
      await userEvent.clear(search);

      // 参加状態と退出。
      expect(within(dialog).getByText("参加中")).toBeTruthy();
      await userEvent.click(within(dialog).getByRole("button", { name: "退出する" }));
      expect(onLeaveThread).toHaveBeenCalledOnce();

      await userEvent.keyboard("{Escape}");
      await waitFor(
        () => expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull(),
        SLOW,
      );
      await waitFor(() => expect(toggle).toHaveFocus(), SLOW);
      expect(onClose).not.toHaveBeenCalled();
    });

    it("閉じたあとに開き直すと、検索語は空に戻る", async () => {
      await page.viewport(1280, 900);
      render(<MembersHarness />);
      const toggle = screen.getByRole("button", { name: "参加者 53人" });
      await userEvent.click(toggle);
      let dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      await userEvent.type(within(dialog).getByRole("searchbox", { name: "名前で検索" }), "car");
      await userEvent.keyboard("{Escape}");
      await waitFor(
        () => expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull(),
        SLOW,
      );

      await userEvent.click(toggle);
      dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      expect(within(dialog).getByRole("searchbox", { name: "名前で検索" })).toHaveValue("");
      expect(within(dialog).getAllByRole("listitem")).toHaveLength(3);
    });

    it("狭幅でも中央のダイアログで、Escape でスレッドのペインを閉じずにダイアログだけを閉じる", async () => {
      await page.viewport(390, 844);
      const onClose = vi.fn();
      render(<MembersHarness onClose={onClose} />);
      await userEvent.click(screen.getByRole("button", { name: "参加者 53人" }));
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      const rect = dialog.getBoundingClientRect();
      expect(rect.left).toBeGreaterThanOrEqual(0);
      expect(rect.right).toBeLessThanOrEqual(window.innerWidth);
      expect(Math.abs(rect.left + rect.width / 2 - window.innerWidth / 2)).toBeLessThan(2);

      await userEvent.keyboard("{Escape}");
      await waitFor(
        () => expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull(),
        SLOW,
      );
      expect(screen.getByRole("dialog", { name: "スレッド" })).toBeVisible();
      expect(onClose).not.toHaveBeenCalled();
    });

    it("人数を読み込めていなくても、参加状態があれば、メンバーのダイアログから退出できる", async () => {
      await page.viewport(1280, 900);
      const onLeaveThread = vi.fn();
      render(
        <LocaleProvider>
          <ThreadPane {...paneProps({ membership: "joined", onLeaveThread })} />
        </LocaleProvider>,
      );
      await userEvent.click(screen.getByRole("button", { name: "メンバー" }));
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      await userEvent.click(within(dialog).getByRole("button", { name: "退出する" }));
      expect(onLeaveThread).toHaveBeenCalledOnce();
    });

    it("参加・退出の失敗は、ダイアログの中に出して、再試行できる", async () => {
      await page.viewport(1280, 900);
      const onMembershipRetry = vi.fn();
      render(
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              membership: "joined",
              participantCount: 3,
              participants,
              onLeaveThread: noop,
              membershipError: true,
              onMembershipRetry,
            })}
          />
        </LocaleProvider>,
      );
      await userEvent.click(screen.getByRole("button", { name: "参加者 3人" }));
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      // 背面のペインの表示ではなく、ダイアログの中に出る（モーダルの背面は操作も読み上げもできない）。
      const alert = within(dialog).getByRole("alert");
      expect(alert.textContent).toMatch(/スレッドの参加状態を更新できませんでした。/);
      await userEvent.click(within(alert).getByRole("button", { name: "再試行" }));
      expect(onMembershipRetry).toHaveBeenCalledOnce();
    });

    it("50人を超えるスレッドの検索は、読み込んだ範囲だけだと表示する", async () => {
      await page.viewport(1280, 900);
      render(<MembersHarness />);
      await userEvent.click(screen.getByRole("button", { name: "参加者 53人" }));
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      // 検索していないあいだは、「ほか N人」。
      expect(within(dialog).getByText("ほか 50人")).toBeTruthy();
      expect(within(dialog).queryByText(/読み込んだ先頭/)).toBeNull();

      // 一覧にいない人を検索すると、見つからない理由を出す。
      await userEvent.type(within(dialog).getByRole("searchbox", { name: "名前で検索" }), "zzz");
      expect(within(dialog).getByText("該当するユーザーがいません。")).toBeTruthy();
      expect(
        within(dialog).getByText(
          "読み込んだ先頭3人の中から絞り込んでいます（ほか 50人は検索できません）。",
        ),
      ).toBeTruthy();
      expect(within(dialog).queryByText("ほか 50人")).toBeNull();
    });

    it("参加状態も人数も未確定のあいだは、メンバーのボタンを出さない", async () => {
      await page.viewport(1280, 900);
      render(
        <LocaleProvider>
          <ThreadPane {...paneProps({ membership: undefined, onLeaveThread: noop })} />
        </LocaleProvider>,
      );
      expect(screen.queryByRole("button", { name: /メンバー|参加者/ })).toBeNull();
    });

    it("未参加では参加ボタンを出し、アーカイブ済みは再Openして参加する（投稿権限がなければ出さない）", async () => {
      await page.viewport(1280, 900);
      const onJoinThread = vi.fn();
      const onReopenAndJoinThread = vi.fn();
      const view = (extra: Partial<ThreadPaneProps>) => (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              membership: "none",
              participantCount: 3,
              participants,
              onJoinThread,
              onReopenAndJoinThread,
              ...extra,
            })}
          />
        </LocaleProvider>
      );
      const { rerender } = render(view({}));
      await userEvent.click(screen.getByRole("button", { name: "参加者 3人" }));
      let dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      expect(within(dialog).getByText("未参加")).toBeTruthy();
      await userEvent.click(within(dialog).getByRole("button", { name: "参加する" }));
      expect(onJoinThread).toHaveBeenCalledOnce();
      await userEvent.keyboard("{Escape}");
      await waitFor(
        () => expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull(),
        SLOW,
      );

      rerender(view({ isArchived: true }));
      await userEvent.click(screen.getByRole("button", { name: "参加者 3人" }));
      dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      await userEvent.click(within(dialog).getByRole("button", { name: "再Openして参加する" }));
      expect(onReopenAndJoinThread).toHaveBeenCalledOnce();
      await userEvent.keyboard("{Escape}");
      await waitFor(
        () => expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull(),
        SLOW,
      );

      rerender(
        view({
          isArchived: true,
          postingForbidden: true,
          composeDisabledReason: "このチャンネルは管理者のみ投稿できます。",
        }),
      );
      await userEvent.click(screen.getByRole("button", { name: "参加者 3人" }));
      dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      expect(within(dialog).queryByRole("button", { name: /参加/ })).toBeNull();
      expect(within(dialog).getByText("未参加")).toBeTruthy();
    });

    it("メンバーでない人や読み取り専用では、参加・退出の操作と、その失敗の表示を出さない", async () => {
      await page.viewport(1280, 900);
      const view = (extra: Partial<ThreadPaneProps>) => (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              membership: "none",
              participantCount: 3,
              participants,
              onJoinThread: noop,
              onLeaveThread: noop,
              // 参加状態の取得に失敗したときにも、エラーは立つ。
              membershipError: true,
              onMembershipRetry: noop,
              ...extra,
            })}
          />
        </LocaleProvider>
      );
      const { rerender } = render(view({ isMember: false }));
      for (const extra of [{ isMember: false }, { readOnly: true }]) {
        rerender(view(extra));
        await userEvent.click(screen.getByRole("button", { name: "参加者 3人" }));
        const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
        expect(within(dialog).queryByRole("button", { name: /参加|退出/ })).toBeNull();
        expect(within(dialog).queryByText("未参加")).toBeNull();
        // ペイン側の表示と同じ条件で、失敗と再試行も出さない。
        expect(within(dialog).queryByRole("alert")).toBeNull();
        expect(within(dialog).queryByRole("button", { name: "再試行" })).toBeNull();
        await userEvent.keyboard("{Escape}");
        await waitFor(
          () => expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull(),
          SLOW,
        );
      }
    });
  });
});
