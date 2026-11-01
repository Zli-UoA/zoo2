import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, within } from "storybook/test";
import { storyChannels, storyUsers } from "../../../.storybook/fixtures";
import { DmSection } from "./DmSection";
const dm = { ...storyChannels[0], id: "dm-1", channel_name: "dm:user-alice:user-bob", is_dm: true };
const meta = {
  title: "Features/DMs/Sidebar",
  component: DmSection,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="w-72 bg-sidebar">
        <Story />
      </div>
    ),
  ],
  args: {
    channels: [dm],
    memberships: [],
    users: storyUsers,
    currentUserId: "user-alice",
    loading: false,
    error: false,
    usersError: false,
    activeChannelId: null,
    unreadChannelIds: new Set<string>(),
    onCreate: fn(),
    onOpen: fn(),
    onReload: fn(),
  },
} satisfies Meta<typeof DmSection>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Empty: Story = {
  args: { channels: [] },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await canvas.findByRole("button", { name: "ダイレクトメッセージ" });
    await expect(canvas.queryByRole("button", { name: "DMを始める" })).not.toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "ダイレクトメッセージ" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    await expect(canvas.getAllByRole("button")).toHaveLength(2);
  }),
};
export const Loading: Story = { args: { channels: [], loading: true } };
export const LoadError: Story = { args: { channels: [], error: true } };
export const UsersError: Story = { args: { usersError: true } };
export const Unread: Story = { args: { unreadChannelIds: new Set([dm.id]) } };
export const Group: Story = {
  args: {
    channels: [{ ...dm, channel_name: "dm:group:story" }],
    memberships: storyUsers.map((user) => ({
      channel_id: dm.id,
      user_id: user.id,
      joined_at: dm.created_at,
    })),
  },
};
