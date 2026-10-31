import type { Decorator, Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, waitFor, within } from "storybook/test";
import { delay, http, HttpResponse } from "msw";
import { useLayoutEffect, type ReactNode } from "react";

import { StoryShell } from "../../../.storybook/shell";
import {
  storyCollections,
  storyNotifications,
  storyChannels,
  storyChannelUsers,
  storyDmChannels,
  storyDmMembers,
  storyDmNotifications,
} from "../../../.storybook/fixtures";
import { NotificationsScreen } from "./NotificationsScreen";
import { ChannelHeader } from "./ChannelHeader";
import { MessageBody } from "./messages/MessageRow";

// 通知カードの自動既読はフォーカス中だけ動く。Storybook では未読の見た目を
// 確認できるよう、この Story の間だけフォーカスを外す。
function UnreadPreview({ children }: { children: ReactNode }) {
  useLayoutEffect(() => {
    const original = document.hasFocus.bind(document);
    document.hasFocus = () => false;
    return () => {
      document.hasFocus = original;
    };
  }, []);
  return <>{children}</>;
}

const withUnreadPreview: Decorator = (Story) => (
  <UnreadPreview>
    <Story />
  </UnreadPreview>
);

const meta = {
  title: "Pages/Notifications",
  component: NotificationsScreen,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  play: async () => {
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={null} initialPane="chat">
        <div className="h-[720px] overflow-hidden bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof NotificationsScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  decorators: [withUnreadPreview],
  play: async () => {
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeVisible();
    const list = screen.getByRole("list", { name: "通知" });
    const region = screen.getByRole("region", { name: "通知" });
    await expect(list.getBoundingClientRect().width).toBe(region.getBoundingClientRect().width);
    for (const row of Array.from(list.children))
      await expect(row.getBoundingClientRect().width).toBe(list.getBoundingClientRect().width);
    await expect(within(list).getByRole("link", { name: "example.com/docs" })).toHaveAttribute(
      "target",
      "_blank",
    );
  },
};

export const SingleUnread: Story = {
  parameters: {
    collections: {
      ...storyCollections,
      notifications: storyNotifications.map((notification, index) => ({
        ...notification,
        is_read: index !== 0,
      })),
    },
  },
};

export const Empty: Story = {
  parameters: {
    collections: { ...storyCollections, notifications: [] },
  },
};

export const Mobile: Story = {
  play: Default.play,
  globals: { viewport: { value: "mobile" } },
  decorators: [withUnreadPreview],
};

export const DirectMessages: Story = {
  decorators: [withUnreadPreview],
  parameters: {
    collections: {
      ...storyCollections,
      channels: [...storyChannels, ...storyDmChannels],
      channelUsers: [...storyChannelUsers, ...storyDmMembers],
      notifications: storyDmNotifications,
    },
  },
  play: async () => {
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(await screen.findByText("Admin, Bob, Carol +1")).toBeVisible();
    await expect(await screen.findByText("Bob", { selector: "p > span" })).toBeVisible();
    await expect(screen.queryByText(/#dm:/)).not.toBeInTheDocument();
  },
};

export const Dark: Story = { ...Default, globals: { theme: "dark" } };

export const MessageLayoutComparison: Story = {
  decorators: [withUnreadPreview],
  render: () => (
    <div className="grid h-full grid-cols-1 grid-rows-2 md:grid-cols-2 md:grid-rows-1">
      <section aria-label="メッセージ表示の比較用">
        <ChannelHeader channel={storyChannels[0]!} memberCount={3} />
        <div className="p-3">
          <MessageBody content="通知とメッセージの本文サイズ・行間を比較します。" />
        </div>
      </section>
      <NotificationsScreen />
    </div>
  ),
  play: async () => {
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    const notification = screen.getByRole("region", { name: "通知" });
    const message = screen.getByRole("region", { name: "メッセージ表示の比較用" });
    const notificationHeader = notification.querySelector("header")!;
    const messageHeader = message.querySelector("header")!;
    await expect(notificationHeader.getBoundingClientRect().height).toBe(
      messageHeader.getBoundingClientRect().height,
    );
    await expect(getComputedStyle(within(notification).getByRole("heading")).fontSize).toBe(
      getComputedStyle(within(message).getByRole("heading")).fontSize,
    );
    const notificationBody = getComputedStyle(notification.querySelector(".mfm-body")!);
    const messageBody = getComputedStyle(message.querySelector(".mfm-body")!);
    await expect(notificationBody.fontSize).toBe(messageBody.fontSize);
    await expect(notificationBody.lineHeight).toBe(messageBody.lineHeight);
    await expect(getComputedStyle(notification.querySelector("time")!).fontSize).toBe("11px");
  },
};

export const MessageLayoutComparisonMobile: Story = {
  ...MessageLayoutComparison,
  globals: { viewport: { value: "mobile" } },
};

export const Loading: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", async () => {
          await delay("infinite");
          return HttpResponse.json({ message: null, reactions: [] });
        }),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("status")).toBeVisible();
    const list = within(screen.getByRole("region", { name: "通知" })).getByRole("list", {
      hidden: true,
    });
    await expect(list).toHaveAttribute("inert");
    await expect(list).not.toBeVisible();
    await waitFor(() => expect(list.querySelectorAll(":scope > [data-unread]")).toHaveLength(3));
  },
};
export const BodiesReadyTogether: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", async ({ params }) => {
          await delay(params.id === "notification-mention" ? 400 : 20);
          return HttpResponse.json({
            message: {
              id: String(params.id),
              user_id: "user-bob",
              content: `本文 ${String(params.id)}`,
            },
            reactions: [],
          });
        }),
      ],
    },
  },
  play: async () => {
    await expect(await screen.findByRole("status")).toBeVisible();
    await expect(
      within(screen.getByRole("region", { name: "通知" })).getByRole("list", { hidden: true }),
    ).not.toBeVisible();
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(screen.getByRole("list", { name: "通知" })).toBeVisible();
    await expect(screen.getByText("本文 notification-mention")).toBeVisible();
    await expect(screen.getAllByText(/^本文 notification-/)).toHaveLength(4);
  },
};

export const LoadingOnScroll: Story = {
  decorators: [withUnreadPreview],
  parameters: {
    collections: {
      ...storyCollections,
      notifications: Array.from({ length: 20 }, (_, index) => ({
        ...storyNotifications[0]!,
        id: `scroll-${index}`,
        message_id: `scroll-message-${index}`,
        created_at: new Date(Date.UTC(2026, 0, 2, 9, 0, 20 - index)).toISOString(),
      })),
    },
    msw: {
      handlers: [
        http.get("*/api/notifications/:id/context", async ({ params }) => {
          if (Number(String(params.id).slice("scroll-".length)) >= 10) await delay("infinite");
          return HttpResponse.json({
            message: {
              id: String(params.id),
              user_id: "user-bob",
              content: `本文 ${String(params.id)}`,
            },
            reactions: [],
          });
        }),
      ],
    },
  },
  play: async () => {
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    const list = screen.getByRole("list", { name: "通知" });
    const scrollRoot = list.parentElement!;
    await expect(screen.getByText("本文 scroll-0")).toBeVisible();
    scrollRoot.scrollTop = scrollRoot.scrollHeight;
    const spinner = await screen.findByRole("status");
    await expect(spinner).toBeVisible();
    await expect(list).toBeVisible();
    await expect(list).not.toHaveAttribute("inert");
    await expect(list).not.toHaveAttribute("aria-hidden");
    await expect(screen.getByText("本文 scroll-0")).toBeInTheDocument();
    await expect(spinner.previousElementSibling).toBe(list);
    await expect(spinner.getBoundingClientRect().bottom).toBeLessThanOrEqual(
      scrollRoot.getBoundingClientRect().bottom,
    );
    scrollRoot.scrollTop = 0;
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(screen.getByText("本文 scroll-0")).toBeVisible();
  },
};
