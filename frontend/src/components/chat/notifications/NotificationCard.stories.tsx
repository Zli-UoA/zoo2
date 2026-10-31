import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen } from "storybook/test";
import { useRef, type ComponentProps } from "react";

import { StoryShell } from "../../../../.storybook/shell";
import {
  storyChannels,
  storyNotifications,
  storyUsers,
  storyDmChannels,
  storyDmMembers,
  storyDmNotifications,
} from "../../../../.storybook/fixtures";
import type { ChannelRow, UserRow } from "../../../lib/electric/types";
import { NotificationCard } from "./NotificationCard";
import type { NotificationGroup } from "./groupNotifications";

const users = new Map<string, UserRow>(storyUsers.map((user) => [user.id, user]));
const channels = new Map<string, ChannelRow>(storyChannels.map((channel) => [channel.id, channel]));

function group(notificationId: string): NotificationGroup {
  const notification = storyNotifications.find((row) => row.id === notificationId)!;
  return {
    key: notification.id,
    latest: notification,
    notifications: [notification],
    actorIds: [notification.actor_id],
  };
}

function NotificationCardDemo(props: Omit<ComponentProps<typeof NotificationCard>, "scrollRoot">) {
  const scrollRoot = useRef<HTMLDivElement>(null);
  return (
    <StoryShell>
      <div ref={scrollRoot} className="mx-auto max-w-3xl bg-background p-6">
        <ul>
          <NotificationCard {...props} scrollRoot={scrollRoot} />
        </ul>
      </div>
    </StoryShell>
  );
}

const meta = {
  title: "Features/Notifications/Card",
  component: NotificationCard,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    channel: { control: false },
    users: { control: false },
  },
  args: {
    group: group("notification-mention"),
    channel: channels.get("channel-general"),
    users,
    onRead: fn(),
    scrollRoot: { current: null },
  },
  render: (args) => <NotificationCardDemo {...args} />,
} satisfies Meta<typeof NotificationCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Mention: Story = {
  play: async () => {
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeInTheDocument();
  },
};

export const Reply: Story = {
  args: {
    group: group("notification-reply"),
  },
};

export const Reaction: Story = {
  args: {
    group: group("notification-reaction"),
  },
};

export const Read: Story = {
  args: {
    group: group("notification-reaction-read"),
  },
};

export const DirectMessage: Story = {
  args: {
    channel: storyDmChannels[0],
    group: {
      key: "notification-dm",
      latest: storyDmNotifications[0],
      notifications: [storyDmNotifications[0]],
      actorIds: ["user-bob"],
    },
  },
  play: async () => {
    await expect(await screen.findByText("Bob", { selector: "p > span" })).toBeVisible();
    await expect(screen.queryByText(/#dm:/)).not.toBeInTheDocument();
    await expect(await screen.findByRole("link", { name: /個別に確認/ })).toHaveAttribute(
      "href",
      expect.stringContaining("/channels/channel-dm?"),
    );
  },
};
export const GroupDirectMessage: Story = {
  args: {
    channel: storyDmChannels[1],
    members: storyDmMembers.filter((row) => row.channel_id === storyDmChannels[1].id),
    group: {
      key: "notification-dm-group",
      latest: storyDmNotifications[1],
      notifications: [storyDmNotifications[1]],
      actorIds: ["user-bob"],
    },
  },
  play: async () => {
    await expect(await screen.findByText("Admin, Bob, Carol +1")).toBeVisible();
    await expect(screen.queryByText(/dm:group:/)).not.toBeInTheDocument();
  },
};
export const DmMetadataPending: Story = {
  ...DirectMessage,
  args: { ...DirectMessage.args, users: new Map() },
  play: async () => {
    await expect(
      await screen.findByText("ダイレクトメッセージ", { selector: "p > span" }),
    ).toBeVisible();
    await expect(screen.queryByText(/#dm:/)).not.toBeInTheDocument();
  },
};
