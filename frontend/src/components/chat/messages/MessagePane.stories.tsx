import { http, HttpResponse } from "msw";
import { storyCollections } from "../../../../.storybook/fixtures";
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

let separateReactionRequests = 0;
export const FederatedReactions: Story = {
  beforeEach: () => {
    separateReactionRequests = 0;
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channels/:id/federated-reactions", () => {
          separateReactionRequests++;
          return HttpResponse.json([]);
        }),
      ],
    },
    sharedApi: true,
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      messages: storyCollections.messages?.map((row) =>
        row.id === federatedReactionFixture.message_id
          ? {
              ...row,
              display_data: {
                emojis: {},
                attachments: [],
                reply_count: 2,
                reactions: [
                  ...(storyCollections.reactions ?? []).filter(
                    (reaction) => reaction.message_id === row.id,
                  ),
                  federatedReactionFixture,
                ],
              },
            }
          : row,
      ),
    },
  },
  play: playForImplementation(async () => {
    await expect(
      await screen.findByRole("button", { name: "👍 3 件（自分もリアクション済み）" }),
    ).toBeVisible();
    const users = screen.getAllByRole("button", { name: "👍 にリアクションしたユーザーを表示" });
    await userEvent.click(users[0]!);
    await waitFor(() => expect(screen.getByText("Peer (@peer@remote.example)")).toBeVisible());
    await expect(separateReactionRequests).toBe(0);
  }),
};

export const Encrypted: Story = {
  args: { encrypted: true },
  play: playForImplementation(async () => {
    // 暗号化の送信を入れるまで、平文を送らないよう入力欄を止め、添付ボタンも出さない。
    const composer = await screen.findByPlaceholderText(
      "このバージョンでは、暗号化チャンネルにまだ送信できません。",
    );
    await expect(composer).toBeDisabled();
    await expect(screen.queryByRole("button", { name: "ファイルを添付" })).not.toBeInTheDocument();
  }),
};

export const EncryptionDowngraded: Story = {
  args: { encrypted: false, channelName: "edm:user-alice:user-bob" },
  play: playForImplementation(async () => {
    // 暗号化 DM の名前なのに encrypted = false が届いたら、平文では送らず警告する。
    const composer = await screen.findByPlaceholderText(
      "このチャンネルは暗号化されていましたが、サーバーが暗号化されていないと返しています。平文では送信しません。",
    );
    await expect(composer).toBeDisabled();
  }),
};
