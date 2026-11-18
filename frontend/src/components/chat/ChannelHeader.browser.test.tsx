import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vite-plus/test";

import { DEFAULT_CONFIG } from "../../lib/config";
import { encryptionSendState, resetSeenEncryptedChannels } from "../../lib/e2ee/encryptionGuard";
import type { ChannelRow } from "../../lib/electric/types";
import type { UserPinnedChannelResponse } from "../../lib/api/types";
import { PinnedChannelsProvider } from "../../hooks/usePinnedChannels";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { ChannelHeader } from "./ChannelHeader";

const channel: ChannelRow = {
  id: "00000000-0000-4000-8000-000000000001",
  channel_name: "general",
  description: null,
  created_user_id: "user-alice",
  is_private: false,
  is_dm: false,
  created_at: "2026-01-01 00:00:00.000",
  updated_at: null,
  archive_at: null,
  imported_from_slack: false,
  slack_import_kind: null,
  slack_imported_at: null,
  deleted_at: null,
};

const encryptApi = vi.hoisted(() => ({ encryptChannel: vi.fn() }));

function pinRow(channelId: string, position: number): UserPinnedChannelResponse {
  return {
    user_id: "me",
    channel_id: channelId,
    position,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: "2026-01-01 00:00:00.000",
  };
}

vi.mock("../../lib/api/encryptChannel", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/api/encryptChannel")>()),
  encryptChannel: encryptApi.encryptChannel,
}));
// 実際のピン留めの Provider を使うため、取得と書き込みだけを差し替える。
const pinsApi = vi.hoisted(() => ({
  pins: [] as UserPinnedChannelResponse[],
  get: vi.fn(),
  post: vi.fn(),
  del: vi.fn(),
}));
// `userPinnedChannels` は同じ関数を返し続ける。毎回新しい関数を返すと、実際の
// `usePinnedChannels` がコレクションの変更とみなして状態を取り直してしまう。
const pinCollection = vi.hoisted(() => ({ factory: () => ({}) }));
const adminState = vi.hoisted(() => ({ isArchived: false }));
vi.mock("@tanstack/react-db", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-db")>()),
  useLiveQuery: () => ({ data: [], isLoading: false, isPersistedReady: true, isError: false }),
}));
vi.mock("../../hooks/useCurrentUser", () => ({
  useCurrentUser: () => ({ currentUserId: "me", isLoading: false }),
}));
vi.mock("../../lib/api/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/api/client")>();
  return {
    ...actual,
    createConfiguredApiClient: () => ({
      get: pinsApi.get,
      post: pinsApi.post,
      patch: vi.fn(),
      delete: pinsApi.del,
    }),
  };
});
vi.mock("../../lib/electric/collections-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/electric/collections-context")>()),
  useCollections: () => ({
    config: { ...DEFAULT_CONFIG, txidMode: "awaitTxId" },
    userPinnedChannels: pinCollection.factory,
    channels: {
      utils: {
        // Shape の反映がいつまでも届かない（待ちが失敗する）場合。
        awaitTxId: () => Promise.reject(new Error("shape timeout")),
      },
    },
  }),
}));

vi.mock("../../hooks/useChannelAdmin", () => ({
  describeApiError: (cause: unknown) => String(cause),
  useChannelAdmin: () => ({
    access: {
      members: [],
      isMember: true,
      isCreator: true,
      isAdmin: false,
      canRead: true,
      canAddMember: true,
      canManageChannel: true,
      isArchived: adminState.isArchived,
      isLoading: false,
      isMembershipPending: false,
      isError: false,
    },
    pending: null,
    setArchived: async () => true,
    kickMember: async () => true,
    deleteMessage: async () => true,
  }),
}));

vi.mock("../../hooks/useRealtime", () => ({
  useMessageFeed: () => ({
    data: [
      {
        id: "message-1",
        channel_id: "00000000-0000-4000-8000-000000000001",
        user_id: "user-bob",
        content: "今日のリリースは 18:00 です。",
        created_at: "2026-01-02 09:00:00.000",
        updated_at: null,
        deleted_at: null,
        parent_id: null,
        user_name: "bob",
        display_name: "Bob",
      },
    ],
    isLoading: false,
    isError: false,
  }),
}));

vi.mock("../layout/shell-context", () => ({
  useShell: () => ({
    pane: "chat",
    showSidebar: vi.fn(),
    showChat: vi.fn(),
    membersOpen: false,
    setMembersOpen: vi.fn(),
    toggleMembers: vi.fn(),
    threadRootId: null,
    openThread: vi.fn(),
    closeThread: vi.fn(),
    selectedChannelId: "00000000-0000-4000-8000-000000000001",
  }),
}));

describe("ChannelHeader", () => {
  beforeEach(() => {
    adminState.isArchived = false;
    pinsApi.pins = [];
    pinsApi.get.mockReset().mockImplementation(async () => ({ pins: pinsApi.pins, version: 1 }));
    pinsApi.post.mockReset();
    pinsApi.del.mockReset();
  });

  it("falls back to the no-description label for missing or whitespace descriptions", () => {
    const { rerender } = render(
      <LocaleProvider>
        <ChannelHeader channel={channel} memberCount={3} />
      </LocaleProvider>,
    );
    expect(screen.getByText("説明はまだありません")).toBeVisible();

    rerender(
      <LocaleProvider>
        <ChannelHeader channel={{ ...channel, description: "   " }} memberCount={3} />
      </LocaleProvider>,
    );
    expect(screen.getByText("説明はまだありません")).toBeVisible();
  });

  it("renders a long description without overflowing the header", () => {
    const description = "長い説明です。".repeat(40);
    render(
      <LocaleProvider>
        <ChannelHeader channel={{ ...channel, description }} memberCount={3} />
      </LocaleProvider>,
    );
    expect(screen.getByText(description)).toBeVisible();
  });

  it("opens channel settings without a separate message moderation screen", async () => {
    render(
      <LocaleProvider>
        <ChannelHeader channel={channel} memberCount={3} />
      </LocaleProvider>,
    );
    await userEvent.click(screen.getByRole("button", { name: "チャンネル設定" }));
    expect(await screen.findByRole("dialog")).toBeVisible();
    expect(screen.getByText("Webhook設定")).toBeVisible();
    expect(screen.getByRole("button", { name: "アーカイブ" })).toBeVisible();
    expect(screen.queryByText("メッセージ管理")).toBeNull();
  });

  it("shows how and when a channel was imported from Slack in the settings only", async () => {
    const imported: ChannelRow = {
      ...channel,
      imported_from_slack: true,
      slack_import_kind: "created",
      slack_imported_at: "2026-11-10 12:00:00.000",
    };
    const { rerender } = render(
      <LocaleProvider>
        <ChannelHeader channel={imported} memberCount={3} />
      </LocaleProvider>,
    );
    // ヘッダーには出さない
    expect(screen.queryByText("Slack から取り込み")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "チャンネル設定" }));
    expect(
      await screen.findByText(/Slack から取り込み・新規作成（最後の取り込み 2026\/.+）/),
    ).toBeInTheDocument();

    // 既存のチャンネルへの取り込み
    rerender(
      <LocaleProvider>
        <ChannelHeader channel={{ ...imported, slack_import_kind: "existing" }} memberCount={3} />
      </LocaleProvider>,
    );
    expect(
      await screen.findByText(/Slack から取り込み・既存（最後の取り込み 2026\/.+）/),
    ).toBeInTheDocument();

    // 取り込んでいないチャンネルの設定には出さない
    rerender(
      <LocaleProvider>
        <ChannelHeader channel={channel} memberCount={3} />
      </LocaleProvider>,
    );
    expect(screen.queryByText(/Slack から/)).toBeNull();
  });

  describe("暗号化への切り替え (#367)", () => {
    const privateA: ChannelRow = {
      ...channel,
      id: "channel-a",
      channel_name: "a",
      is_private: true,
    };
    const privateB: ChannelRow = {
      ...channel,
      id: "channel-b",
      channel_name: "b",
      is_private: true,
    };

    it("確認を開いたまま対象のチャンネルが変わったら、確認を閉じて移動先を切り替えない", async () => {
      encryptApi.encryptChannel.mockReset();
      const { rerender } = render(
        <LocaleProvider>
          <ChannelHeader channel={privateA} memberCount={3} />
        </LocaleProvider>,
      );
      await userEvent.click(screen.getByRole("button", { name: "チャンネル設定" }));
      await userEvent.click(await screen.findByRole("button", { name: "暗号化を有効にする" }));
      expect(await screen.findByText("このチャンネルを暗号化しますか？")).toBeVisible();

      rerender(
        <LocaleProvider>
          <ChannelHeader channel={privateB} memberCount={3} />
        </LocaleProvider>,
      );
      await waitFor(() =>
        expect(screen.queryByText("このチャンネルを暗号化しますか？")).toBeNull(),
      );
      expect(encryptApi.encryptChannel).not.toHaveBeenCalled();
    });

    it("API が暗号化を返したら、Shape の反映を待てなくても送信側が暗号化として扱う", async () => {
      resetSeenEncryptedChannels();
      encryptApi.encryptChannel.mockReset();
      encryptApi.encryptChannel.mockResolvedValue({
        data: { ...privateA, encrypted: true, membership_epoch: 1 },
        txid: "42",
      });
      render(
        <LocaleProvider>
          <ChannelHeader channel={privateA} memberCount={3} />
        </LocaleProvider>,
      );
      await userEvent.click(screen.getByRole("button", { name: "チャンネル設定" }));
      await userEvent.click(await screen.findByRole("button", { name: "暗号化を有効にする" }));
      await userEvent.click(await screen.findByRole("button", { name: "暗号化する" }));
      await waitFor(() =>
        expect(screen.queryByText("このチャンネルを暗号化しますか？")).toBeNull(),
      );

      expect(encryptApi.encryptChannel).toHaveBeenCalledWith(expect.anything(), "channel-a");
      // Shape はまだ `encrypted = false` のまま。それでも平文の送信を止める状態になる。
      expect(
        encryptionSendState({
          channelId: "channel-a",
          encrypted: false,
          known: "unavailable",
          hasCiphertext: false,
        }).downgraded,
      ).toBe(true);
    });
  });
});

describe("ピン留めのフォーカス", () => {
  beforeEach(() => {
    adminState.isArchived = false;
    pinsApi.pins = [];
    pinsApi.get.mockReset().mockImplementation(async () => ({ pins: pinsApi.pins, version: 1 }));
    pinsApi.post.mockReset();
    pinsApi.del.mockReset();
  });

  it("keeps focus on the header pin button while the write is pending, and ignores extra activation", async () => {
    const pinned = pinRow(channel.id, 0);
    // POST は 200ms 待ってから成功する。
    pinsApi.post.mockImplementation(
      () =>
        new Promise((resolve) => {
          setTimeout(
            () => resolve({ data: { pin: pinned, version: 2, pins: [pinned] }, txid: "2" }),
            200,
          );
        }),
    );
    render(
      <LocaleProvider>
        <PinnedChannelsProvider>
          <ChannelHeader channel={channel} memberCount={3} />
        </PinnedChannelsProvider>
      </LocaleProvider>,
    );

    const pinButton = await screen.findByRole("button", { name: "ピン留め" });
    pinButton.focus();
    await userEvent.keyboard("{Enter}");

    // 書き込み中もネイティブの `disabled` は付けず、フォーカスを保ったまま
    // `aria-disabled` で二重操作を防ぐ。
    await waitFor(() => expect(pinButton.getAttribute("aria-disabled")).toBe("true"));
    expect(document.activeElement).toBe(pinButton);
    await waitFor(() => expect(pinsApi.post).toHaveBeenCalledTimes(1));

    // 保留中にもう一度押しても、2 本目の POST は送らない。
    await userEvent.keyboard("{Enter}");

    // 完了しても同じボタンにフォーカスが残る。
    await waitFor(() => expect(pinButton.getAttribute("aria-disabled")).not.toBe("true"));
    expect(document.activeElement).toBe(pinButton);
    expect(screen.getByRole("button", { name: "ピン留めを解除" })).toBe(pinButton);
    // 保留中の押下が積まれていれば、ここまでに 2 本目が送られている。
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(pinsApi.post).toHaveBeenCalledTimes(1);
  });

  it("moves focus to a remaining header action when unpinning an archived channel removes the pin button", async () => {
    adminState.isArchived = true;
    pinsApi.pins = [pinRow(channel.id, 0)];
    // DELETE は 200ms 待ってから成功する。
    pinsApi.del.mockImplementation(
      () =>
        new Promise((resolve) => {
          setTimeout(
            () => resolve({ data: { removed: true, version: 2, pins: [] }, txid: "2" }),
            200,
          );
        }),
    );
    render(
      <LocaleProvider>
        <PinnedChannelsProvider>
          <ChannelHeader
            channel={{ ...channel, archive_at: "2026-01-02 00:00:00.000" }}
            memberCount={3}
          />
        </PinnedChannelsProvider>
      </LocaleProvider>,
    );

    const pinButton = await screen.findByRole("button", { name: "ピン留めを解除" });
    pinButton.focus();
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(pinButton.getAttribute("aria-disabled")).toBe("true"));
    expect(document.activeElement).toBe(pinButton);

    // アーカイブ中のチャンネルは、解除するとピンボタン自体が消える。フォーカスが
    // `document.body` へ落ちないよう、残っているヘッダーの操作へ移す。
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "ピン留めを解除" })).toBeNull(),
    );
    await waitFor(() => {
      const focused = document.activeElement;
      expect(focused).not.toBe(document.body);
      expect(focused).toBe(screen.getByRole("button", { name: "メンバー 3" }));
    });
  });

  it("does not move focus to another header action when switching channels hides the pin button", async () => {
    const view = (current: ChannelRow) => (
      <LocaleProvider>
        <PinnedChannelsProvider>
          <ChannelHeader channel={current} memberCount={3} />
        </PinnedChannelsProvider>
      </LocaleProvider>
    );
    const { rerender } = render(view(channel));
    const pinButton = await screen.findByRole("button", { name: "ピン留め" });
    pinButton.focus();
    expect(document.activeElement).toBe(pinButton);

    // アーカイブ中でピン留めされていない別チャンネルへ切り替えると、ピンボタンが消える。
    // 同じ画面内の切り替えでは、残っているヘッダー操作へフォーカスを奪わない。
    adminState.isArchived = true;
    rerender(
      view({
        ...channel,
        id: "other-channel",
        channel_name: "other",
        archive_at: "2026-01-02 00:00:00.000",
      }),
    );
    await waitFor(() => expect(screen.queryByRole("button", { name: "ピン留め" })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(document.body));
  });
});
