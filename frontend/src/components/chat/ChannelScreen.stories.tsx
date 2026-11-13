import { playForImplementation } from "../../../.storybook/play-for-implementation";
import { http, HttpResponse } from "msw";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";
import { storyCommands } from "../../../.storybook/app-fixtures";
import type { Meta, StoryObj } from "@storybook/react-vite";

import { StoryShell } from "../../../.storybook/shell";
import { storyAttachments } from "../../../.storybook/attachment-fixtures";
import { storyCollections } from "../../../.storybook/fixtures";
import type { MessageRow } from "../../lib/electric/types";
import { ChannelScreen } from "./ChannelScreen";

const general = storyCollections.channels.find((channel) => channel.id === "channel-general")!;
const threadChannel = {
  ...general,
  id: "thread-release-planning",
  channel_name: "thread-thread-release-planning",
  parent_channel_id: general.id,
  thread_title: "Release planning",
  thread_archived_at: null,
  thread_activity_at: general.created_at,
};

function replyScenarioMessages(
  channelId: string,
  {
    replyToReply = false,
    multipleReplies = false,
  }: { replyToReply?: boolean; multipleReplies?: boolean } = {},
): MessageRow[] {
  const users = storyCollections.users ?? [];
  const alice = users.find((user) => user.id === "user-alice")!;
  const bob = users.find((user) => user.id === "user-bob")!;
  const carol = users.find((user) => user.id === "user-carol")!;
  const parent: MessageRow = {
    id: `${channelId}-reply-parent`,
    user_id: alice.id,
    channel_id: channelId,
    content: "リリース計画を確認してください。切替は18時の予定です。",
    created_at: "2026-01-02 09:00:00.000",
    updated_at: null,
    deleted_at: null,
    parent_id: null,
  };
  const makeReply = (
    id: string,
    user: typeof bob,
    content: string,
    target: MessageRow,
    seconds: number,
  ): MessageRow => ({
    id: `${channelId}-${id}`,
    user_id: user.id,
    channel_id: channelId,
    content,
    created_at: `2026-01-02 09:00:${String(seconds).padStart(2, "0")}.000`,
    updated_at: null,
    deleted_at: null,
    parent_id: null,
    reply_to_message_id: target.id,
    display_data: {
      emojis: {},
      attachments: [],
      reactions: [],
      reply_count: 0,
      reply_target: {
        id: target.id,
        user_id: target.user_id,
        channel_id: target.channel_id,
        content: target.content,
        deleted_at: null,
        author: {
          user_name:
            users.find((candidate) => candidate.id === target.user_id)?.user_name ?? "user",
          display_name:
            users.find((candidate) => candidate.id === target.user_id)?.display_name ?? "User",
          avatar_url: null,
        },
      },
    },
  });
  const first = makeReply(
    "reply-bob",
    bob,
    "切替手順を確認しました。",
    parent,
    12,
  );
  if (!replyToReply && !multipleReplies) return [parent, first];
  const second = makeReply(
    "reply-carol",
    carol,
    replyToReply ? "その補足を反映しました。" : "監視項目を追加しました。",
    replyToReply ? first : parent,
    24,
  );
  return [parent, first, second];
}

function withReplyMessages(
  channelId: string,
  options: { replyToReply?: boolean; multipleReplies?: boolean } = {},
) {
  return {
    ...storyCollections,
    messages: [
      ...(storyCollections.messages ?? []).filter((message) => message.channel_id !== channelId),
      ...replyScenarioMessages(channelId, options),
    ],
    channels:
      channelId === threadChannel.id
        ? [...(storyCollections.channels ?? []), threadChannel]
        : storyCollections.channels,
    channelUsers:
      channelId === threadChannel.id
        ? [
            ...(storyCollections.channelUsers ?? []),
            ...["user-alice", "user-bob", "user-carol"].map((user_id) => ({
              user_id,
              channel_id: threadChannel.id,
              joined_at: general.created_at,
            })),
          ]
        : storyCollections.channelUsers,
  };
}

const meta = {
  title: "Pages/Channel",
  component: ChannelScreen,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  args: {
    channelId: "channel-general",
  },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId="channel-general">
        <div className="h-[720px] min-w-0 flex-1 bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
} satisfies Meta<typeof ChannelScreen>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const ThreadChannel: Story = {
  args: { channelId: threadChannel.id },
  parameters: {
    collections: {
      ...storyCollections,
      channels: [...storyCollections.channels, threadChannel],
    },
  },
};

export const RegularChannelReplies: Story = {
  parameters: { collections: withReplyMessages(general.id) },
};

export const ThreadChannelReplies: Story = {
  args: { channelId: threadChannel.id },
  parameters: { collections: withReplyMessages(threadChannel.id) },
};

export const ReplyToReply: Story = {
  parameters: { collections: withReplyMessages(general.id, { replyToReply: true }) },
};

export const MultipleRepliesToOneMessage: Story = {
  parameters: { collections: withReplyMessages(general.id, { multipleReplies: true }) },
};

export const PrivateChannel: Story = {
  args: {
    channelId: "channel-secret",
  },
};

export const Archived: Story = {
  args: {
    channelId: "channel-archive",
  },
};

export const NotFound: Story = {
  args: {
    channelId: "channel-missing",
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  decorators: [
    (Story) => (
      <StoryShell selectedChannelId="channel-general" initialPane="chat">
        <div className="h-[720px] min-w-0 flex-1 bg-background">
          <Story />
        </div>
      </StoryShell>
    ),
  ],
};

const commandLookup = fn();
const commandInvocation = fn();
export const CommandCompletion: Story = {
  parameters: {
    a11y: { test: "error" },
    msw: {
      handlers: [
        http.get("*/api/apps/commands", ({ request }) => {
          commandLookup(new URL(request.url).searchParams.get("channel_id"));
          return HttpResponse.json(storyCommands);
        }),
        http.post("*/api/apps/:id/invoke", async ({ params, request }) => {
          commandInvocation(params.id, await request.json());
          return HttpResponse.json({ event_id: "command-event" });
        }),
      ],
    },
  },
  beforeEach: () => {
    commandLookup.mockClear();
    commandInvocation.mockClear();
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const input = await canvas.findByRole("textbox");
    await userEvent.type(input, "/ec");
    await userEvent.click(await canvas.findByRole("option", { name: "/echo Meeting Notes" }));
    // Composer restores the caret on the next animation frame after completion.
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    await userEvent.type(input, "hello");
    await userEvent.click(canvas.getByRole("button", { name: "送信" }));
    await waitFor(() =>
      expect(commandInvocation).toHaveBeenCalledWith("notes", {
        kind: "command",
        name: "echo",
        channel_id: "channel-general",
        values: { text: "hello" },
      }),
    );
    await expect(commandLookup).toHaveBeenCalledWith("channel-general");
    await expect(input).toHaveValue("");
  }),
};

export const Nsfw: Story = {
  parameters: {
    a11y: { test: "error" },
    collections: {
      ...storyCollections,
      attachments: [{ ...storyAttachments[0], message_id: "message-hello", is_sensitive: true }],
      channels: storyCollections.channels.map((channel) => ({
        ...channel,
        is_nsfw: channel.id === "channel-general",
      })),
    },
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const reveal = await canvas.findByRole("button", { name: "画像を表示" });
    await expect(reveal.querySelector("canvas")).not.toBeNull();
    await expect(canvas.queryByRole("img", { name: "screenshot.png" })).not.toBeInTheDocument();
    await userEvent.click(reveal);
    await waitFor(() => expect(canvas.getByRole("img", { name: "screenshot.png" })).toBeVisible());
  }),
};
