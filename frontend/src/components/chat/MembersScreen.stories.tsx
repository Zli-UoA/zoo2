import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, within } from "storybook/test";
import { storyChannels, storyCollections, storyChannelUsers } from "../../../.storybook/fixtures";
import { MembersPanel } from "./MembersPanel";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;
const meta = {
  title: "Pages/ChannelMembers",
  component: MembersPanel,
  parameters: { a11y: { test: "error" } },
  globals: { viewport: { value: "mobile" } },
  args: { channel: general, onBack: fn(), className: "flex h-full" },
  decorators: [
    (Story) => (
      <div className="h-dvh bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MembersPanel>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("heading", { name: "メンバー", level: 1 })).toBeVisible();
    await expect(canvas.getByRole("button", { name: "チャンネルに戻る" })).toBeVisible();
    await expect(canvas.queryByRole("dialog")).not.toBeInTheDocument();
  }),
};
export const PublicNotJoined: Story = {
  parameters: {
    collections: {
      ...storyCollections,
      channelUsers: storyChannelUsers.filter(
        (row) => !(row.channel_id === general.id && row.user_id === "user-alice"),
      ),
    },
  },
};
