import type { Meta, StoryObj } from "@storybook/react-vite";

import { StoryShell } from "../../../.storybook/shell";
import { ChannelScreen } from "./ChannelScreen";

const meta = {
  title: "Pages/Channel",
  component: ChannelScreen,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    channelId: "channel-general",
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId="channel-general">
        <div className="h-[720px] min-w-0 flex-1 bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const PrivateChannel: Story = {
  args: {
    channelId: "channel-secret",
  },
};

export const Archived: Story = {
  args: {
    channelId: "channel-archive",
  },
};

export const NotFound: Story = {
  args: {
    channelId: "channel-missing",
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId="channel-general" initialPane="chat">
        <div className="h-[720px] min-w-0 flex-1 bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
};
