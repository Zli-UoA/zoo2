import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";
import { storyChannels } from "../../../.storybook/fixtures";
import { DmListItem } from "./DmListItem";
const meta = {
  title: "Features/DMs/ListItem",
  component: DmListItem,
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <div className="w-72 bg-sidebar p-2">
        <Story />
      </div>
    ),
  ],
  args: {
    channel: { ...storyChannels[0], id: "dm-story", channel_name: "dm:alice:bob", is_dm: true },
    label: "Bob",
    active: false,
    onOpen: fn(),
  },
} satisfies Meta<typeof DmListItem>;
export default meta;
type Story = StoryObj<typeof meta>;
export const Default: Story = {};
export const Active: Story = { args: { active: true } };
export const Unread: Story = { args: { unread: true } };
export const LongName: Story = {
  args: { label: "とても長い名前のメンバーとのダイレクトメッセージ" },
};
