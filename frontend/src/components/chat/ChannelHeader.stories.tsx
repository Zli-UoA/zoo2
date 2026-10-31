import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor, within } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { storyChannels, storyChannelUsers } from "../../../.storybook/fixtures";
import { ChannelHeader } from "./ChannelHeader";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;
const secret = storyChannels.find((channel) => channel.id === "channel-secret")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;
const memberCount = (channelId: string) =>
  storyChannelUsers.filter((row) => row.channel_id === channelId).length;

const meta = {
  title: "Features/Channels/Header",
  component: ChannelHeader,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    memberCount: { control: { type: "number", min: 0 } },
  },
  args: {
    channel: general,
    memberCount: memberCount(general.id),
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId={general.id}>
        <div className="w-full bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const ControlPanelOpen: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(screen.getByRole("button", { name: "チャンネル設定" }));
    await expect(await screen.findByRole("dialog")).toBeInTheDocument();
  }),
};

export const Private: Story = {
  args: {
    channel: secret,
    memberCount: memberCount(secret.id),
  },
};

export const Archived: Story = {
  parameters: { a11y: { test: "todo" } },
  args: {
    channel: archived,
    memberCount: memberCount(archived.id),
  },
};

export const LoadingMemberCount: Story = {
  args: {
    memberCount: null,
  },
};

export const NoDescription: Story = {
  args: {
    channel: { ...general, description: null },
  },
};

export const LongDescription: Story = {
  args: {
    channel: {
      ...general,
      description:
        "長い説明の表示確認用チャンネルです。目的・参加対象・運用ルールをまとめています。\n二行目も含めて崩れないことを確認します。",
    },
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};

export const WebhookSettingsOpen: Story = {
  play: playForImplementation(async () => {
    await userEvent.click(screen.getByRole("button", { name: "チャンネル設定" }));
    const settings = await screen.findByRole("dialog", { name: "チャンネル設定" });
    await userEvent.click(within(settings).getByRole("button", { name: /^開く$/ }));
    await waitFor(async () => {
      await expect(screen.getByRole("dialog", { name: "Webhook設定" })).toBeVisible();
    });
    await waitFor(async () => {
      await expect(
        screen.queryByRole("dialog", { name: "チャンネル設定" }),
      ).not.toBeInTheDocument();
    });
    await userEvent.keyboard("{Escape}");
    await waitFor(async () => {
      await expect(screen.queryByRole("dialog", { name: "Webhook設定" })).not.toBeInTheDocument();
    });
  }),
};
