import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent } from "storybook/test";
import type { ReactNode } from "react";

import { StoryShell } from "../../../../.storybook/shell";
import { MessagePane } from "./MessagePane";

function PaneFrame({ children }: { children: ReactNode }) {
  return (
    <StoryShell selectedChannelId="channel-general">
      <div className="flex h-[720px] min-w-0 flex-1 bg-background">
        <div className="flex min-w-0 flex-1 flex-col">{children}</div>
      </div>
    </StoryShell>
  );
}

const meta = {
  title: "Chat/MessagePane",
  component: MessagePane,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    isMember: { control: "boolean" },
  },
  args: {
    channelId: "channel-general",
    isMember: true,
  },
  render: (args) => (
    <PaneFrame>
      <MessagePane {...args} />
    </PaneFrame>
  ),
} satisfies Meta<typeof MessagePane>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const ThreadOpen: Story = {
  play: async () => {
    const replies = await screen.findByRole("button", { name: "2 件の返信（スレッドを開く）" });
    await userEvent.click(replies);
    await expect(
      await screen.findByRole("complementary", { name: "スレッド" }),
    ).toBeInTheDocument();
  },
};

export const NotJoined: Story = {
  args: {
    isMember: false,
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};
