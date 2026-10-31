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

  it("uses a machine-readable datetime in the message moderation dialog", async () => {
    render(
      <LocaleProvider>
        <ChannelHeader channel={channel} memberCount={3} />
      </LocaleProvider>,
    );

    await userEvent.click(screen.getByRole("button", { name: "チャンネルの管理" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "メッセージ管理" }));

    const dialog = await screen.findByRole("dialog");
    const time = dialog.querySelector("time");
    expect(time?.getAttribute("datetime")).toBe("2026-01-02T09:00:00.000Z");
  });
});
