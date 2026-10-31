import { BubbleChatScreen } from "../chat/BubbleChatScreen";
import { bubbleChatHandlers } from "../../../.storybook/bubble-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, waitFor } from "storybook/test";

import { storyCollections, storyNotifications } from "../../../.storybook/fixtures";
import { ChannelScreen } from "../chat/ChannelScreen";
import { NotificationsScreen } from "../chat/NotificationsScreen";
import { WelcomePanel } from "../chat/WelcomePanel";
import { AppShell } from "./AppShell";

const meta = {
  title: "Pages/Shell",
  component: AppShell,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    children: <WelcomePanel />,
  },
} satisfies Meta<typeof AppShell>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Welcome: Story = {
  play: async () => {
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).toBeVisible();
  },
};

export const Channel: Story = {
  args: {
    children: <ChannelScreen channelId="channel-general" />,
  },
  parameters: {
    router: { initialEntries: ["/channels/channel-general"] },
  },
  play: async () => {
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeInTheDocument();
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).toBeVisible();
  },
};

export const MobileChannel: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    children: <ChannelScreen channelId="channel-general" />,
  },
  parameters: {
    router: { initialEntries: ["/channels/channel-general"] },
  },
};

export const Notifications: Story = {
  args: {
    children: <NotificationsScreen />,
  },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/notifications"] },
  },
  play: async () => {
    await waitFor(() => expect(screen.queryByRole("status")).not.toBeInTheDocument());
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).not.toBeVisible();
    const main = screen.getByRole("main").getBoundingClientRect();
    const nav = screen.getByRole("navigation").getBoundingClientRect();
    await expect(main.x).toBe(nav.right);
    await expect(main.right).toBe(document.documentElement.clientWidth);
  },
};

export const NotificationsSingleUnread: Story = {
  args: {
    children: <NotificationsScreen />,
  },
  parameters: {
    router: { initialEntries: ["/notifications"] },
    collections: {
      ...storyCollections,
      notifications: storyNotifications.map((notification, index) => ({
        ...notification,
        is_read: index !== 0,
      })),
    },
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    children: <WelcomePanel />,
  },
};

export const MobileNotifications: Story = {
  ...Notifications,
  globals: { viewport: { value: "mobile" } },
};

export const BubbleChat: Story = {
  args: { children: <BubbleChatScreen /> },
  parameters: {
    a11y: { test: "error" },
    router: { initialEntries: ["/bubble"] },
    msw: { handlers: bubbleChatHandlers },
  },
  play: async () => {
    await expect(
      await screen.findByRole("textbox", { name: "リアルタイムメッセージ入力" }),
    ).toBeVisible();
    await expect(screen.getByLabelText("チャンネル", { selector: "aside" })).not.toBeVisible();
    await expect(screen.getByRole("link", { name: "Bubble Chat" })).toHaveAttribute(
      "data-status",
      "active",
    );
  },
};
