import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, fn, screen } from "storybook/test";

import { storyMessages, storyReactions, storyUsers } from "../../../../.storybook/fixtures";
import { groupReactions } from "../../../lib/emoji/reactions";
import { MessageList } from "./MessageList";
import type { FeedMessage } from "./messageFormat";

const users = new Map(storyUsers.map((user) => [user.id, user]));

function feedMessage(id: string): FeedMessage {
  const row = storyMessages.find((message) => message.id === id)!;
  const user = users.get(row.user_id);
  return {
    id: row.id,
    user_id: row.user_id,
    content: row.content,
    card: row.card,
    created_at: row.created_at,
    parent_id: row.parent_id,
    user_name: user?.user_name ?? null,
    display_name: user?.display_name ?? null,
  };
}

const channelMessages = storyMessages
  .filter((message) => message.channel_id === "channel-general" && message.parent_id === null)
  .map((message) => feedMessage(message.id));

const longMessages: FeedMessage[] = [
  {
    id: "message-long-en",
    user_id: "user-bob",
    content:
      "The release checklist now includes the migration dry run, the rollback rehearsal, and a final verification of the audit log. Please review the attached notes before the deployment window opens and leave any blockers in the thread so we can address them early.",
    created_at: "2026-01-02 11:00:00.000",
    parent_id: null,
    user_name: "bob",
    display_name: "Bob",
  },
  {
    id: "message-long-zh",
    user_id: "user-carol",
    content:
      "本次发布的检查清单包括数据库迁移演练、回滚演练以及审计日志的最终确认。请在发布窗口开始前查看附带的说明，并在讨论串中留下任何阻塞项，以便我们尽早处理。",
    created_at: "2026-01-02 11:01:00.000",
    parent_id: null,
    user_name: "carol",
    display_name: "Carol",
  },
];

const meta = {
  title: "Chat/MessageList",
  component: MessageList,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    isLoading: { control: "boolean" },
    isError: { control: "boolean" },
    canReact: { control: "boolean" },
    canCompose: { control: "boolean" },
  },
  args: {
    messages: channelMessages,
    isLoading: false,
    isError: false,
    currentUserId: "user-alice",
    editingId: null,
    editingText: "",
    onEditingTextChange: fn(),
    onStartEdit: fn(),
    onCancelEdit: fn(),
    onSaveEdit: fn(),
    onRequestDelete: fn(),
    reactions: groupReactions(storyReactions, "user-alice"),
    kitchenCatalog: null,
    canReact: true,
    onReaction: fn(),
    onPickReaction: fn(),
    replyCounts: new Map([["message-thread-root", 2]]),
    onOpenThread: fn(),
  },
  decorators: [
    (Story) => (
      <div className="mx-auto flex h-[640px] max-w-3xl flex-col bg-background">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MessageList>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Empty: Story = {
  args: {
    messages: [],
  },
};

export const Loading: Story = {
  args: {
    messages: [],
    isLoading: true,
  },
};

export const Error: Story = {
  args: {
    messages: [],
    isError: true,
  },
};

export const UnreadSeparator: Story = {
  args: {
    channelId: "channel-general",
  },
  parameters: {
    msw: {
      handlers: [
        http.get("*/api/channels/:channelId/read-state", ({ params }) =>
          HttpResponse.json({
            channel_id: String(params.channelId),
            last_read_at: "2026-01-02 09:00:01.000",
            has_unread: true,
          }),
        ),
      ],
    },
  },
  play: async () => {
    // アプリ側の aria-label は英語のまま。Story で検出できるよう名前を固定する。
    await expect(await screen.findByRole("separator", { name: "new" })).toBeInTheDocument();
  },
};

export const LongMessages: Story = {
  args: {
    messages: longMessages,
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    messages: channelMessages.slice(0, 3),
  },
};
