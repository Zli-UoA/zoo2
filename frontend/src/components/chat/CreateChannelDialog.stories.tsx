import { playForImplementation } from "../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";

import { CreateChannelDialog } from "./CreateChannelDialog";

const meta = {
  title: "Features/Channels/Create",
  component: CreateChannelDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    open: { control: "boolean" },
  },
  args: {
    open: true,
    onOpenChange: fn(),
  },
} satisfies Meta<typeof CreateChannelDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Filled: Story = {
  play: playForImplementation(async () => {
    await userEvent.type(screen.getByPlaceholderText("新しいチャンネル名"), "new-channel");
    await userEvent.type(
      screen.getByPlaceholderText("このチャンネルの目的や話題"),
      "新しいチャンネルの説明",
    );
    await userEvent.click(screen.getByRole("checkbox", { name: "非公開にする" }));
    await expect(screen.getByRole("button", { name: "チャンネル作成" })).toBeEnabled();
  }),
};

export const Closed: Story = {
  args: {
    open: false,
  },
};
