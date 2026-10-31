import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";

import { storyChannels } from "../../../.storybook/fixtures";
import { RenameChannelDialog } from "./RenameChannelDialog";

const general = storyChannels.find((channel) => channel.id === "channel-general")!;

const meta = {
  title: "Features/Channels/Edit",
  component: RenameChannelDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  args: {
    channel: general,
    onOpenChange: fn(),
  },
} satisfies Meta<typeof RenameChannelDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Filled: Story = {
  play: async () => {
    const input = screen.getByRole("textbox", { name: "新しいチャンネル名" });
    await userEvent.clear(input);
    await userEvent.type(input, "general-renamed");
    const description = screen.getByRole("textbox", { name: "説明（任意）" });
    await userEvent.clear(description);
    await userEvent.type(description, "チーム全体の連絡用チャンネル");
    await expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
  },
};

export const EmptyDescription: Story = {
  args: {
    channel: { ...general, description: null },
  },
};
