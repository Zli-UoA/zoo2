import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vite-plus/test";

import type { ChannelRow } from "../../lib/electric/types";
import { LocaleProvider } from "../i18n/LocaleProvider";
import { ChannelHeader } from "./ChannelHeader";

const channel: ChannelRow = {
  id: "channel-general",
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
        channel_id: "channel-general",
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
    selectedChannelId: "channel-general",
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
      await screen.findByText(/Slack から新しいチャンネルとして取り込みました（2026\/.+）。/),
    ).toBeInTheDocument();

    // 既存のチャンネルへの取り込み
    rerender(
      <LocaleProvider>
        <ChannelHeader channel={{ ...imported, slack_import_kind: "existing" }} memberCount={3} />
      </LocaleProvider>,
    );
    expect(
      await screen.findByText(/Slack から既存のチャンネルに取り込みました（2026\/.+）。/),
    ).toBeInTheDocument();

    // 取り込んでいないチャンネルの設定には出さない
    rerender(
      <LocaleProvider>
        <ChannelHeader channel={channel} memberCount={3} />
      </LocaleProvider>,
    );
    expect(screen.queryByText(/Slack から/)).toBeNull();
  });
});
