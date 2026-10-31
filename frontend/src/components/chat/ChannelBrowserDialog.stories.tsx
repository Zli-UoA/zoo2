import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent } from "storybook/test";

import { StoryShell } from "../../../.storybook/shell";
import { storyChannels } from "../../../.storybook/fixtures";
import { ChannelBrowserDialog } from "./ChannelBrowserDialog";

const joined = new Set(["channel-general", "channel-design", "channel-secret"]);

const meta = {
  title: "Screens/ChannelBrowserDialog",
  component: ChannelBrowserDialog,
  tags: ["autodocs"],
  parameters: { a11y: { test: "error" } },
  argTypes: {
    open: { control: "boolean" },
  },
  args: {
    open: true,
    onOpenChange: fn(),
    channels: storyChannels,
    joinedChannelIds: joined,
  },
  decorators: [
    (Story) => (
      <StoryShell>
        <Story />
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelBrowserDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  play: async () => {
    await expect(await screen.findByText("random")).toBeInTheDocument();
    await expect(screen.getByRole("link", { name: /random/ })).toBeInTheDocument();
  },
};

export const SearchEmpty: Story = {
  play: async () => {
    await userEvent.type(screen.getByRole("searchbox"), "zzzz");
    await expect(screen.getByText("一致するチャンネルがありません。")).toBeInTheDocument();
  },
};

export const Closed: Story = {
  args: {
    open: false,
  },
};
