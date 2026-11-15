import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";

import { DEFAULT_CONFIG } from "../../lib/config";
import { encryptionSendState, resetSeenEncryptedChannels } from "../../lib/e2ee/encryptionGuard";
import type { ChannelRow } from "../../lib/electric/types";
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
vi.mock("../../lib/api/encryptChannel", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/api/encryptChannel")>()),
  encryptChannel: encryptApi.encryptChannel,
}));
vi.mock("../../lib/electric/collections-context", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/electric/collections-context")>()),
  useCollections: () => ({
    config: { ...DEFAULT_CONFIG, txidMode: "awaitTxId" },
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
      isArchived: false,
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
