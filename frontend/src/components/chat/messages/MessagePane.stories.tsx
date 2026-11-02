import { http, HttpResponse } from "msw";
import { federatedReactionFixture } from "../../../../.storybook/message-fixtures";
import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";
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
  title: "Features/Chat/MessagePane",
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

export const Default: Story = {
  play: playForImplementation(async () => {
    await expect(await screen.findByText(/今日のリリースは 18:00/)).toBeVisible();
    await expect(screen.queryByRole("heading", { name: "メッセージ" })).not.toBeInTheDocument();
  }),
};

export const ThreadOpen: Story = {
  play: playForImplementation(async () => {
    const replies = await screen.findByRole("button", { name: "2 件の返信（スレッドを開く）" });
    await userEvent.click(replies);
    await expect(
      await screen.findByRole("complementary", { name: "スレッド" }),
    ).toBeInTheDocument();
  }),
};

export const NotJoined: Story = {
  args: {
    isMember: false,
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
};

export const FederatedReactions: Story = {
  parameters: {
    sharedApi: true,
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/channels/:id/federated-reactions", () =>
          HttpResponse.json([federatedReactionFixture]),
        ),
      ],
    },
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("button", { name: "👍 3 件（自分もリアクション済み）" }),
    ).toBeVisible();
    const users = screen.getAllByRole("button", { name: "👍 にリアクションしたユーザーを表示" });
    await userEvent.click(users[0]!);
    await waitFor(() => expect(screen.getByText("Peer (@peer@remote.example)")).toBeVisible());
  }),
};
