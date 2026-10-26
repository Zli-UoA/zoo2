import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";

import { StoryShell } from "../../../.storybook/shell";
import { storyChannelUsers, storyChannels, storyCollections } from "../../../.storybook/fixtures";
import { ChannelSidebar } from "./ChannelSidebar";

const meta = {
  title: "Screens/ChannelSidebar",
  component: ChannelSidebar,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    loading: { control: "boolean" },
    error: { control: "boolean" },
  },
  args: {
    channels: storyChannels,
    loading: false,
    error: false,
  },
  decorators: [
    (Story) => (
      <StoryShell>
        <div className="flex h-[640px] bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelSidebar>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Empty: Story = {
  args: {
    channels: [],
  },
};

export const Loading: Story = {
  args: {
    loading: true,
  },
};

export const LoadError: Story = {
  args: {
    error: true,
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    channels: storyChannels,
  },
};

export const UnreadAndArchived: Story = {
  args: {
    channels: storyChannels,
  },
  parameters: {
    collections: {
      ...storyCollections,
      channelUsers: [
        ...storyChannelUsers,
        {
          user_id: "user-alice",
          channel_id: "channel-archive",
          joined_at: "2026-01-01 00:00:00.000",
        },
      ],
    },
    msw: {
      handlers: [
        http.get("*/api/channel-read-states", () =>
          HttpResponse.json([
            { channel_id: "channel-general", last_read_at: null, has_unread: true },
            { channel_id: "channel-design", last_read_at: null, has_unread: true },
          ]),
        ),
      ],
    },
  },
};
