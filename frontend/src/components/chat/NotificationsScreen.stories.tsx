import type { Decorator, Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen } from "storybook/test";
import { useLayoutEffect, type ReactNode } from "react";

import { StoryShell } from "../../../.storybook/shell";
import { storyCollections, storyNotifications } from "../../../.storybook/fixtures";
import { NotificationsScreen } from "./NotificationsScreen";

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
  title: "Notifications/NotificationsScreen",
  component: NotificationsScreen,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
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
    await expect(await screen.findByText("未読通知 3 件")).toBeInTheDocument();
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeInTheDocument();
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
  globals: { viewport: { value: "mobile" } },
  decorators: [withUnreadPreview],
};
