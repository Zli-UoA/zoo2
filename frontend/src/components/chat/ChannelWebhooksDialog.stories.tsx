import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { fn } from "storybook/test";

import { storyChannels } from "../../../.storybook/fixtures";
import { ChannelWebhooksDialog } from "./ChannelWebhooksDialog";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;

const meta = {
  title: "Screens/ChannelWebhooksDialog",
  component: ChannelWebhooksDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    channel: general,
    onOpenChange: fn(),
  },
} satisfies Meta<typeof ChannelWebhooksDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Empty: Story = {
  parameters: {
    msw: {
      handlers: [http.get("*/api/channels/:channelId/webhooks", () => HttpResponse.json([]))],
    },
  },
};

export const Archived: Story = {
  args: {
    channel: archived,
  },
};

export const LoadError: Story = {
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channels/:channelId/webhooks", () =>
          HttpResponse.json({ error: "failed" }, { status: 500 }),
        ),
      ],
    },
  },
};
