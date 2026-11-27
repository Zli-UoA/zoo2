import "../../../styles.css";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vite-plus/test";

import type { ResolvedForwardMessage } from "../../../lib/api/types";
import type { ChannelRow, ChannelUserRow, UserRow } from "../../../lib/electric/types";
import { LocaleProvider } from "../../i18n/LocaleProvider";
import { ForwardMessageDialog, buildForwardDestinations } from "./ForwardMessageDialog";

const USERS: UserRow[] = [
  {
    id: "u-me",
    user_name: "me",
    display_name: "Me",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
  {
    id: "u-bob",
    user_name: "bob",
    display_name: "Bob",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
  {
    id: "u-carol",
    user_name: "carol",
    display_name: "Carol",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
  {
    id: "u-dave",
    user_name: "dave",
    display_name: "Dave",
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
  },
];

const CHANNELS: ChannelRow[] = [
  {
    id: "ch-pub",
    channel_name: "general",
    description: null,
    created_user_id: "u-bob",
    is_private: false,
    is_nsfw: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
    posting_policy: "members",
  },
  {
    id: "ch-priv",
    channel_name: "secret-team",
    description: null,
    created_user_id: "u-bob",
    is_private: true,
    is_nsfw: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
    posting_policy: "members",
  },
  {
    id: "ch-enc",
    channel_name: "e2ee-room",
    description: null,
    created_user_id: "u-me",
    is_private: true,
    is_nsfw: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: true,
  },
  {
    id: "ch-arch",
    channel_name: "archived",
    description: null,
    created_user_id: "u-me",
    is_private: false,
    is_nsfw: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: "2026-01-02 00:00:00.000",
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
  },
  {
    id: "ch-mgr",
    channel_name: "announcements",
    description: null,
    created_user_id: "u-bob",
    is_private: false,
    is_nsfw: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
    posting_policy: "managers",
  },
  {
    id: "ch-unjoined",
    channel_name: "other",
    description: null,
    created_user_id: "u-bob",
    is_private: false,
    is_nsfw: false,
    is_dm: false,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
  },
  {
    id: "ch-dm",
    channel_name: "dm:me:bob",
    description: null,
    created_user_id: "u-me",
    is_private: true,
    is_nsfw: false,
    is_dm: true,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
  },
  {
    id: "ch-gdm",
    channel_name: "dm:group:1",
    description: null,
    created_user_id: "u-me",
    is_private: true,
    is_nsfw: false,
    is_dm: true,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
  },
  {
    id: "ch-gdm-2",
    channel_name: "dm:group:2",
    description: null,
    created_user_id: "u-me",
    is_private: true,
    is_nsfw: false,
    is_dm: true,
    created_at: "2026-01-01 00:00:00.000",
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    encrypted: false,
  },
];

const MEMBERSHIPS: ChannelUserRow[] = [
  { user_id: "u-me", channel_id: "ch-pub", joined_at: "", notification_level: "mentions" },
  { user_id: "u-me", channel_id: "ch-priv", joined_at: "", notification_level: "mentions" },
  { user_id: "u-me", channel_id: "ch-enc", joined_at: "", notification_level: "mentions" },
  { user_id: "u-me", channel_id: "ch-arch", joined_at: "", notification_level: "mentions" },
  { user_id: "u-me", channel_id: "ch-mgr", joined_at: "", notification_level: "mentions" },
  { user_id: "u-me", channel_id: "ch-dm", joined_at: "", notification_level: "mentions" },
  { user_id: "u-bob", channel_id: "ch-dm", joined_at: "", notification_level: "mentions" },
  { user_id: "u-me", channel_id: "ch-gdm", joined_at: "", notification_level: "mentions" },
  { user_id: "u-bob", channel_id: "ch-gdm", joined_at: "", notification_level: "mentions" },
  { user_id: "u-carol", channel_id: "ch-gdm", joined_at: "", notification_level: "mentions" },
  { user_id: "u-me", channel_id: "ch-gdm-2", joined_at: "", notification_level: "mentions" },
  { user_id: "u-bob", channel_id: "ch-gdm-2", joined_at: "", notification_level: "mentions" },
  { user_id: "u-dave", channel_id: "ch-gdm-2", joined_at: "", notification_level: "mentions" },
];

const SOURCE: ResolvedForwardMessage = {
  id: "msg-1",
  channel_id: "ch-pub",
  channel_name: "general",
  is_private: false,
  is_dm: false,
  dm_participants: [],
  parent_id: null,
  user_id: "u-bob",
  created_at: "2026-01-02 20:20:00.000",
  updated_at: null,
  content: "あああてすと",
  card: null,
  components: null,
  message_kind: "message",
  author: {
    id: "u-bob",
    user_name: "bob",
    display_name: "Bob",
    avatar_url: null,
    is_bot: false,
    app_id: null,
  },
  emojis: {},
  attachments: [],
  has_forwards: false,
};

describe("buildForwardDestinations", () => {
  it("excludes encrypted, archived, unjoined, and non-writable managers-only channels", () => {
    const t = ((k: string) => k) as never;
    const memberDests = buildForwardDestinations({
      channels: CHANNELS,
      memberships: MEMBERSHIPS,
      users: USERS,
      currentUserId: "u-me",
      isAdmin: false,
      locale: "ja",
      t,
    });
    expect(memberDests.map((d) => d.id).sort()).toEqual([
      "ch-dm",
      "ch-gdm",
      "ch-gdm-2",
      "ch-priv",
      "ch-pub",
    ]);
    expect(memberDests.find((d) => d.id === "ch-gdm")?.label).toBe("Bob, Carol");
    expect(memberDests.find((d) => d.id === "ch-gdm-2")?.label).toBe("Bob, Dave");

    const adminDests = buildForwardDestinations({
      channels: CHANNELS,
      memberships: MEMBERSHIPS,
      users: USERS,
      currentUserId: "u-me",
      isAdmin: true,
      locale: "ja",
      t,
    });
    expect(adminDests.map((d) => d.id).sort()).toEqual([
      "ch-dm",
      "ch-gdm",
      "ch-gdm-2",
      "ch-mgr",
      "ch-priv",
      "ch-pub",
    ]);
  });
});

describe("ForwardMessageDialog", () => {
  beforeEach(() => {
    localStorage.setItem("zoo-locale", "ja");
  });

  afterEach(() => {
    localStorage.clear();
  });

  const destinations = [
    { id: "ch-pub", label: "general", isDm: false, isPrivate: false },
    { id: "ch-priv", label: "secret-team", isDm: false, isPrivate: true },
    { id: "ch-dm", label: "Bob", isDm: true, isPrivate: true },
    { id: "ch-gdm", label: "Bob, Carol", isDm: true, isPrivate: true },
    { id: "ch-gdm-2", label: "Bob, Dave", isDm: true, isPrivate: true },
  ];

  it("filters destinations, allows empty-comment forward, clears selection, and copies link", async () => {
    const onSubmit = vi.fn();
    const copy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    try {
      render(
        <LocaleProvider>
          <ForwardMessageDialog
            open
            onOpenChange={() => {}}
            source={SOURCE}
            destinations={destinations}
            onSubmit={onSubmit}
          />
        </LocaleProvider>,
      );

      const submitBtn = screen.getByRole("button", { name: "転送する" });
      expect(submitBtn).toBeDisabled();

      const searchInput = screen.getByRole("combobox", { name: "転送先" });
      await userEvent.type(searchInput, "secret");
      expect(screen.queryByRole("option", { name: "general" })).toBeNull();
      await userEvent.click(screen.getByRole("option", { name: "secret-team" }));

      expect(submitBtn).toBeEnabled();
      await userEvent.click(screen.getByRole("button", { name: "メッセージリンクをコピー" }));
      await waitFor(() => {
        expect(copy).toHaveBeenCalledWith(
          expect.stringContaining("/channels/ch-pub?message=msg-1"),
        );
      });

      await userEvent.click(submitBtn);
      expect(onSubmit).toHaveBeenCalledWith("ch-priv", "");

      // 選択を解除して別の転送先 + コメント付きで送信
      await userEvent.click(screen.getByRole("button", { name: "secret-team を選択から外す" }));
      await userEvent.click(screen.getByRole("option", { name: "general" }));
      await userEvent.type(screen.getByRole("textbox", { name: "コメント" }), "てんそうしたよ");
      await userEvent.click(screen.getByRole("button", { name: "転送する" }));
      expect(onSubmit).toHaveBeenLastCalledWith("ch-pub", "てんそうしたよ");
    } finally {
      copy.mockRestore();
    }
  });

  it("ignores Enter in destination search during IME composition", async () => {
    render(
      <LocaleProvider>
        <ForwardMessageDialog
          open
          onOpenChange={() => {}}
          source={SOURCE}
          destinations={destinations}
          onSubmit={() => {}}
        />
      </LocaleProvider>,
    );

    const searchInput = screen.getByRole("combobox", { name: "転送先" });
    const submitBtn = screen.getByRole("button", { name: "転送する" });

    fireEvent.keyDown(searchInput, { key: "Enter", isComposing: true });
    fireEvent.keyDown(searchInput, { key: "Enter", keyCode: 229 });
    expect(submitBtn).toBeDisabled();

    fireEvent.keyDown(searchInput, { key: "Enter", isComposing: false });
    expect(submitBtn).toBeEnabled();
  });

  it("distinguishes multiple group DMs by participants, moves focus to comment box, and announces selection", async () => {
    render(
      <LocaleProvider>
        <ForwardMessageDialog
          open
          onOpenChange={() => {}}
          source={SOURCE}
          destinations={destinations}
          onSubmit={() => {}}
        />
      </LocaleProvider>,
    );

    const searchInput = screen.getByRole("combobox", { name: "転送先" });
    expect(screen.getByRole("option", { name: "Bob, Carol" })).toBeVisible();
    expect(screen.getByRole("option", { name: "Bob, Dave" })).toBeVisible();

    await userEvent.type(searchInput, "Dave");
    expect(screen.queryByRole("option", { name: "Bob, Carol" })).toBeNull();
    await userEvent.click(screen.getByRole("option", { name: "Bob, Dave" }));

    const commentBox = screen.getByRole("textbox", { name: "コメント" });
    await waitFor(() => {
      expect(commentBox).toHaveFocus();
    });
    expect(screen.getByRole("status")).toHaveTextContent("Bob, Dave を転送先に選択しました");
  });
});
