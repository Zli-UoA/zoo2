import { avatarUsers, userAvatarHandler, userAvatarUrl } from "../../../.storybook/avatar-fixtures";
import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";

import { storyChannels, storyCollections, storyChannelUsers } from "../../../.storybook/fixtures";
import { MembersPanel } from "./MembersPanel";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;
const secret = storyChannels.find((channel) => channel.id === "channel-secret")!;
const archived = storyChannels.find((channel) => channel.id === "channel-archive")!;
const thread = {
  ...secret,
  id: "thread-general",
  channel_name: "thread-general",
  parent_channel_id: secret.id,
  thread_title: "General thread",
};

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

export const ThreadMembers: Story = {
  args: { channel: thread },
  parameters: {
    collections: {
      ...storyCollections,
      channelUsers: [
        ...storyChannelUsers,
        { user_id: "user-bob", channel_id: thread.id, joined_at: "2026-11-13T00:00:00.000Z" },
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(await screen.findByText("Bob")).toBeVisible();
    await expect(screen.queryByRole("combobox", { name: "メンバーを追加" })).not.toBeInTheDocument();
    await expect(screen.queryByRole("button", { name: "参加" })).not.toBeInTheDocument();
    await expect(screen.queryByRole("button", { name: "退出" })).not.toBeInTheDocument();
  }),
};

export const UserAvatar: Story = {
  parameters: {
    collections: { ...storyCollections, users: avatarUsers },
    msw: { handlers: [userAvatarHandler] },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await waitFor(async () => {
      const image = canvasElement.querySelector<HTMLImageElement>(`img[src="${userAvatarUrl}"]`);
      await expect(image?.naturalWidth).toBeGreaterThan(0);
    });
  }),
};
