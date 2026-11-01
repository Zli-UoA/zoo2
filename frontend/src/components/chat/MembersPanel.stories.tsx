import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";

import { storyChannels, storyCollections, storyChannelUsers } from "../../../.storybook/fixtures";
import { MembersPanel } from "./MembersPanel";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;
const secret = storyChannels.find((channel) => channel.id === "channel-secret")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;

const meta = {
  title: "Features/Channels/Members",
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

export const Default: Story = {};

export const AddCandidates: Story = {
  play: playForImplementation(async () => {
    const input = await screen.findByRole("combobox", { name: "メンバーを追加" });
    await userEvent.click(input);
    await expect(screen.getByRole("listbox", { name: "メンバー候補" })).toBeVisible();
  }),
};

export const Archived: Story = {
  args: {
    channel: archived,
  },
};

export const PrivateNotJoined: Story = {
  args: {
    channel: secret,
  },
  parameters: {
    collections: {
      ...storyCollections,
      channelUsers: storyChannelUsers.filter(
        (row) => !(row.channel_id === secret.id && row.user_id === "user-alice"),
      ),
    },
  },
};
