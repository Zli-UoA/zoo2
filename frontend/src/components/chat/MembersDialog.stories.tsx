import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { storyChannels } from "../../../.storybook/fixtures";
import { MembersDialog } from "./MembersDialog";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;

const meta = {
  title: "Features/Channels/MembersDialog",
  component: MembersDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    channel: general,
    open: true,
    onOpenChange: fn(),
  },
} satisfies Meta<typeof MembersDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Closed: Story = {
  args: {
    open: false,
  },
};
