import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen } from "storybook/test";

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

export const Welcome: Story = {};

export const Channel: Story = {
  args: {
    children: <ChannelScreen channelId="channel-general" />,
  },
  parameters: {
    router: { initialEntries: ["/channels/channel-general"] },
  },
  play: async () => {
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeInTheDocument();
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
    router: { initialEntries: ["/notifications"] },
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
