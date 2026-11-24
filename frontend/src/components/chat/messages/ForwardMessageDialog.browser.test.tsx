import "../../../styles.css";
import { render, screen, waitFor } from "@testing-library/react";
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
];

const MEMBERSHIPS: ChannelUserRow[] = [
  { user_id: "u-me", channel_id: "ch-pub", joined_at: "" },
  { user_id: "u-me", channel_id: "ch-priv", joined_at: "" },
  { user_id: "u-me", channel_id: "ch-enc", joined_at: "" },
  { user_id: "u-me", channel_id: "ch-arch", joined_at: "" },
  { user_id: "u-me", channel_id: "ch-mgr", joined_at: "" },
  { user_id: "u-me", channel_id: "ch-dm", joined_at: "" },
  { user_id: "u-bob", channel_id: "ch-dm", joined_at: "" },
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
    expect(memberDests.map((d) => d.id).sort()).toEqual(["ch-dm", "ch-priv", "ch-pub"]);

    const adminDests = buildForwardDestinations({
      channels: CHANNELS,
      memberships: MEMBERSHIPS,
      users: USERS,
      currentUserId: "u-me",
      isAdmin: true,
      locale: "ja",
      t,
    });
    expect(adminDests.map((d) => d.id).sort()).toEqual(["ch-dm", "ch-mgr", "ch-priv", "ch-pub"]);
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
});
