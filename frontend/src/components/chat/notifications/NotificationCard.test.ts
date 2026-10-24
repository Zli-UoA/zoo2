import { createElement, createRef, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { beforeEach, expect, it, vi } from "vite-plus/test";
import { createI18n } from "../../../lib/i18n";
import type {
  ChannelRow,
  MessageRow,
  MessageReactionRow,
  NotificationRow,
  UserRow,
} from "../../../lib/electric/types";
import { NotificationCard } from "./NotificationCard";
import { groupNotifications } from "./groupNotifications";

const state = vi.hoisted(() => ({
  message: null as MessageRow | null,
  reactions: [] as MessageReactionRow[],
  canRead: true,
}));
vi.mock("./useReadVisibleNotification", () => ({
  useReadVisibleNotification: () => ({ ref: { current: null }, hasBeenVisible: true }),
}));
vi.mock("./useNotificationContext", () => ({
  useNotificationContext: () => ({
    data: state.canRead
      ? { message: state.message?.deleted_at ? null : state.message, reactions: state.reactions }
      : null,
    isLoading: false,
  }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    params,
    search,
    children,
    ...props
  }: {
    params: { channelId: string };
    search: { message: string };
    children: ReactNode;
  }) =>
    createElement(
      "a",
      { ...props, href: `/channels/${params.channelId}?message=${search.message}` },
      children,
    ),
}));
vi.mock("../../../hooks/useKitchenCatalog", () => ({
  useKitchenCatalog: () => ({ catalog: null }),
}));
vi.mock("../../i18n/LocaleProvider", () => ({ useLocale: () => ({ locale: "ja" }) }));
vi.mock("../../layout/shell-context", () => ({ useShell: () => ({ showChat: vi.fn() }) }));
const now = "2026-10-24T00:00:00Z";
const users = new Map(
  ["alice", "bob", "me"].map((id): [string, UserRow] => [
    id,
    {
      id,
      user_name: id,
      display_name: id,
      created_at: now,
      updated_at: null,
    },
  ]),
);
const channel: ChannelRow = {
  id: "channel",
  channel_name: "general",
  description: null,
  created_user_id: "me",
  is_private: false,
  created_at: now,
  updated_at: null,
  archive_at: null,
  deleted_at: null,
};
const rows: NotificationRow[] = ["alice", "bob"].map((id) => ({
  id,
  actor_id: id,
  user_id: "me",
  channel_id: "channel",
  message_id: "message",
  notification_type: "reaction",
  is_read: false,
  created_at: now,
}));
function render(notifications = rows) {
  return renderToStaticMarkup(
    createElement(
      I18nextProvider,
      { i18n: createI18n("ja") },
      createElement(NotificationCard, {
        group: groupNotifications(notifications)[0],
        channel,
        users,
        onRead: vi.fn(),
        scrollRoot: createRef<HTMLDivElement>(),
      }),
    ),
  );
}
beforeEach(() => {
  state.canRead = true;
  state.message = {
    id: "message",
    channel_id: "channel",
    user_id: "me",
    content: "長い引用".repeat(100),
    created_at: now,
    updated_at: null,
    deleted_at: null,
    parent_id: null,
  };
  state.reactions = ["alice", "bob"].map((user_id) => ({
    id: user_id,
    user_id,
    message_id: "message",
    emoji: "👍",
    created_at: now,
  }));
});
it("shows one grouped reaction card with a limited clickable quote and no action buttons", () => {
  const html = render();
  expect(html).toContain("ほか1人がリアクションしました");
  expect(html).toContain('aria-label="👍 2"');
  expect(html).toContain('href="/channels/channel?message=message"');
  expect(html).toContain("<blockquote>");
  expect(html).toContain("line-clamp-3");
  expect(html).not.toContain(state.message!.content);
  expect(html).not.toContain("<button");
  expect(html).not.toContain("メッセージを開く");
});
it("does not offer a quote link for deleted or inaccessible messages", () => {
  state.message!.deleted_at = now;
  expect(render()).not.toContain("<blockquote>");
  state.message!.deleted_at = null;
  state.canRead = false;
  const html = render();
  expect(html).not.toContain("<blockquote>");
  expect(html).toContain("閲覧できません");
});

it("labels ordinary direct messages independently from replies", () => {
  const html = render([{ ...rows[0], notification_type: "dm" }]);
  expect(html).toContain("ダイレクトメッセージが届きました");
  expect(html).not.toContain("返信しました");
});
