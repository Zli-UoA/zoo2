import "../../../styles.css";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vite-plus/test";
import { cdp, page } from "vite-plus/test/browser/context";

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

it("regroups replies when a time zone change puts midnight between them", async () => {
  const replies = [
    message({ id: "r1", created_at: "2026-10-19T14:59:00Z" }),
    message({ id: "r2", created_at: "2026-10-19T15:01:00Z" }),
  ];
  try {
    await cdp().send("Emulation.setTimezoneOverride", { timezoneId: "UTC" });
    const view = render(
      <LocaleProvider>
        <ThreadPane {...paneProps({ replies })} />
      </LocaleProvider>,
    );
    const separators = () => view.container.querySelectorAll('[role="separator"][aria-label]');
    expect(separators()).toHaveLength(1);
    await cdp().send("Emulation.setTimezoneOverride", { timezoneId: "Asia/Tokyo" });
    window.dispatchEvent(new Event("focus"));
    await waitFor(() => expect(separators()).toHaveLength(2));
  } finally {
    await cdp().send("Emulation.setTimezoneOverride", { timezoneId: "" });
    window.dispatchEvent(new Event("focus"));
  }
});

it.each(["bottom", "middle", "top"])(
  "preserves the %s scroll position when grouped replies grow at midnight",
  async (position) => {
    vi.useFakeTimers({ toFake: ["Date", "setTimeout", "clearTimeout"] });
    const now = new Date(2026, 9, 19, 23, 59, 59);
    vi.setSystemTime(now);
    try {
      const replies = Array.from({ length: 20 }, (_, i) =>
        message({ id: String(i), created_at: now.toISOString(), content: "reply " + i }),
      );
      const view = render(
        <LocaleProvider>
          <ThreadPane {...paneProps({ replies })} />
        </LocaleProvider>,
      );
      const scroll = view.container.querySelector<HTMLElement>("[data-scroll-restoration-id]")!;
      scroll.style.cssText = "height: 180px; flex: none; overflow-anchor: none";
      const height = scroll.scrollHeight;
      expect(height).toBeGreaterThan(scroll.clientHeight);
      scroll.scrollTop = position === "bottom" ? height : 0;
      const middle = scroll.querySelector<HTMLElement>('[data-message-id="8"]')!;
      const offset = () => middle.getBoundingClientRect().top - scroll.getBoundingClientRect().top;
      if (position === "middle") scroll.scrollTop += offset() + 4;
      const before = offset();
      fireEvent.scroll(scroll);
      await act(() => vi.advanceTimersByTimeAsync(1000));
      expect(scroll.scrollHeight).toBeGreaterThan(height);
      if (position === "middle") expect(Math.abs(offset() - before)).toBeLessThanOrEqual(0.5);
      else
        expect(scroll.scrollTop).toBe(
          position === "bottom" ? scroll.scrollHeight - scroll.clientHeight : 0,
        );
      view.unmount();
    } finally {
      vi.useRealTimers();
    }
  },
);

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
      participantCount = 53,
    }: {
      onClose?: () => void;
      onLeaveThread?: () => void;
      /** 一覧に載る人数（3）より多いと、「ほか N人」の注記が出る。既定は 53。 */
      participantCount?: number;
    }) {
      return (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              // スレ主（親メッセージの投稿者）は bob、自分は alice (`currentUserId` は user-1)。
              root: message({ user_id: "user-2", user_name: "bob", display_name: null }),
              replies: [message({ id: "reply-1", parent_id: "root-1", content: "a reply" })],
              membership: "joined",
              participantCount,
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

    // ダイアログには、ライブリージョンが 2 つある。検索結果（一覧の下。DOM の順で先）と、参加状態（フッター）。
    const searchStatus = (dialog: HTMLElement) => within(dialog).getAllByRole("status")[0]!;
    const joinStatus = (dialog: HTMLElement) => within(dialog).getAllByRole("status")[1]!;

    it("アーカイブ済み・読み取り専用で操作用の ID が null でも、閲覧者の ID で自分に「あなた」の印を付ける", async () => {
      await page.viewport(1280, 900);
      for (const extra of [
        { isArchived: true },
        { readOnly: true },
        { readOnly: true, isMember: false, mentionUserId: null },
      ]) {
        const { unmount } = render(
          <LocaleProvider>
            <ThreadPane
              {...paneProps({
                root: message({ user_id: "user-2", user_name: "bob", display_name: null }),
                replies: [message({ id: "reply-1", parent_id: "root-1", content: "a reply" })],
                membership: "joined",
                participantCount: participants.length,
                participants,
                // MessagePane と同じく、操作を止めるために currentUserId は null にする。
                currentUserId: null,
                viewerId: "user-1",
                ...extra,
              })}
            />
          </LocaleProvider>,
        );
        await userEvent.click(
          screen.getByRole("button", { name: `参加者 ${participants.length}人` }),
        );
        const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
        const rows = within(
          within(dialog).getByRole("list", { name: `参加者 ${participants.length}人` }),
        ).getAllByRole("listitem");
        expect(rows.map((row) => row.textContent)).toEqual([
          expect.stringMatching(/^.*bob.*スレ主$/),
          expect.stringMatching(/^.*Alice.*あなた$/),
          expect.stringMatching(/Carol/),
        ]);
        unmount();
      }
    });

    it("検索結果の 0 件は、常設のライブリージョン（role=status）で伝え、入力欄のフォーカスは動かさない", async () => {
      await page.viewport(1280, 900);
      render(<MembersHarness />);
      await userEvent.click(screen.getByRole("button", { name: "参加者 53人" }));
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      // ライブリージョンは、変化の前から DOM にある（あとから現れる要素は、読み上げられないことがある）。
      const status = searchStatus(dialog);
      expect(status.textContent).toMatch(/ほか 50人/);

      const search = within(dialog).getByRole("searchbox", { name: "名前で検索" });
      await userEvent.type(search, "zzz");
      // 同じ要素の中身が更新される。
      expect(searchStatus(dialog)).toBe(status);
      expect(status.textContent).toMatch(/該当するユーザーがいません。/);
      expect(status.textContent).toMatch(/読み込んだ先頭3人の中から絞り込んでいます/);
      expect(document.activeElement).toBe(search);

      // 一致が戻ると、0 件のメッセージは消える（ライブリージョン自体は残る）。
      await userEvent.clear(search);
      expect(searchStatus(dialog)).toBe(status);
      expect(status.textContent).not.toMatch(/該当するユーザーがいません。/);
    });

    it("全員を読み込めているスレッドでも、ライブリージョンは検索の前から DOM にあり、0 件になっても同じ要素のまま", async () => {
      await page.viewport(1280, 900);
      // 人数（3）が一覧の人数（3）と同じなので、「ほか N人」の注記はない。リージョンの中身は、最初は空。
      render(<MembersHarness participantCount={3} />);
      await userEvent.click(screen.getByRole("button", { name: "参加者 3人" }));
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      const status = searchStatus(dialog);
      expect(status.textContent).toBe("");
      expect(within(dialog).queryByText(/ほか/)).toBeNull();

      const search = within(dialog).getByRole("searchbox", { name: "名前で検索" });
      await userEvent.type(search, "zzz");
      // 0 件のメッセージは、同じ要素の中身として入る（あとから現れる要素ではない）。
      expect(searchStatus(dialog)).toBe(status);
      expect(status.textContent).toBe("該当するユーザーがいません。");
      expect(document.activeElement).toBe(search);

      await userEvent.clear(search);
      expect(searchStatus(dialog)).toBe(status);
      expect(status.textContent).toBe("");
    });

    it("参加状態の文言は、ライブリージョン（role=status）の中で、同じ要素のまま更新される", async () => {
      await page.viewport(1280, 900);
      const view = (membership: "joined" | "none") => (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({
              membership,
              participantCount: 3,
              participants,
              onLeaveThread: noop,
              onJoinThread: noop,
            })}
          />
        </LocaleProvider>
      );
      const { rerender } = render(view("joined"));
      await userEvent.click(screen.getByRole("button", { name: "参加者 3人" }));
      const dialog = await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);
      const status = joinStatus(dialog);
      expect(status.textContent).toBe("参加中");
      // 検索結果の領域とは別の要素で、その領域には手を加えない。
      expect(searchStatus(dialog)).not.toBe(status);

      // 退出が成功して、参加状態が変わる（ダイアログは開いたまま）。同じ要素の中身が更新される。
      rerender(view("none"));
      await waitFor(() => expect(status.textContent).toBe("未参加"), SLOW);
      expect(joinStatus(dialog)).toBe(status);
      expect(status.isConnected).toBe(true);
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

    it("ダイアログを描画しなくなったら、開いた状態も解除する（狭幅のペインの Escape が戻り、あとで勝手に開かない）", async () => {
      await page.viewport(390, 844);
      const onClose = vi.fn();
      const view = (extra: Partial<ThreadPaneProps>) => (
        <LocaleProvider>
          <ThreadPane
            {...paneProps({ membership: "joined", onLeaveThread: noop, onClose, ...extra })}
          />
        </LocaleProvider>
      );
      // 人数が未取得で、参加状態だけがあるので、メンバーのボタンが出る
      const { rerender } = render(view({}));
      await userEvent.click(screen.getByRole("button", { name: "メンバー" }));
      await screen.findByRole("dialog", { name: /メンバー/ }, SLOW);

      // メンバーでなくなると、ボタンもダイアログも描画されない
      rerender(view({ isMember: false }));
      await waitFor(
        () => expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull(),
        SLOW,
      );
      expect(screen.queryByRole("button", { name: "メンバー" })).toBeNull();

      // ダイアログが開いたままの状態が残らないので、狭幅のペインの Escape が、ペインを閉じる
      const pane = screen.getByRole("dialog", { name: "スレッド" });
      pane.focus();
      await userEvent.keyboard("{Escape}");
      expect(onClose).toHaveBeenCalledOnce();

      // メンバーに戻っても、ダイアログは勝手に開かない
      rerender(view({}));
      expect(await screen.findByRole("button", { name: "メンバー" })).toBeTruthy();
      expect(screen.queryByRole("dialog", { name: /メンバー/ })).toBeNull();
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

describe("ThreadPane の削除された親", () => {
  it("親の代わりに知らせを出し、返信は読めるようにする", async () => {
    await page.viewport(1280, 800);
    render(
      <LocaleProvider>
        <ThreadPane
          {...paneProps({
            root: message({ content: "消した親の本文" }),
            rootDeleted: true,
            readOnly: true,
            currentUserId: null,
            replies: [message({ id: "reply-1", parent_id: "root-1", content: "残った返信" })],
          })}
        />
      </LocaleProvider>,
    );
    expect(await screen.findByText("元のメッセージは削除されました")).toBeTruthy();
    expect(screen.getByText("残った返信")).toBeTruthy();
    expect(screen.queryByText("消した親の本文")).toBeNull();
  });
});
