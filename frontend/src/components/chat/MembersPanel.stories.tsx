import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";

import { storyChannels, storyCollections, storyChannelUsers } from "../../../.storybook/fixtures";
import { MembersPanel } from "./MembersPanel";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;
const design = storyChannels.find((channel) => channel.id === "channel-design")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;

const meta = {
  title: "Screens/MembersPanel",
  component: MembersPanel,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    channel: general,
  },
  decorators: [
    (Story) => (
      <div className="h-[640px] w-80 bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MembersPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: async () => {
    const input = screen.getByRole("combobox", { name: "メンバーを追加" });
    await userEvent.click(input);
    await expect(screen.getByRole("listbox", { name: "メンバー候補" })).toBeVisible();
  },
};

export const Archived: Story = {
  args: {
    channel: archived,
  },
};

export const PrivateNotJoined: Story = {
  args: {
    channel: design,
  },
  parameters: {
    collections: {
      ...storyCollections,
      channelUsers: storyChannelUsers.filter((row) => row.channel_id !== "channel-design"),
    },
  },
};
