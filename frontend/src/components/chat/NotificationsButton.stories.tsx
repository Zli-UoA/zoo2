import type { Meta, StoryObj } from "@storybook/react-vite";

import { storyCollections, storyNotifications } from "../../../.storybook/fixtures";
import { NotificationsButton } from "./NotificationsButton";

const meta = {
  title: "Notifications/NotificationsButton",
  component: NotificationsButton,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="flex items-center bg-background p-6">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof NotificationsButton>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Unread: Story = {};

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

export const AllRead: Story = {
  parameters: {
    collections: {
      ...storyCollections,
      notifications: storyNotifications.map((notification) => ({
        ...notification,
        is_read: true,
      })),
    },
  },
};
