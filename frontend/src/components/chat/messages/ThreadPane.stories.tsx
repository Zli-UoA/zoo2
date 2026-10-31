import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { ThreadPane } from "./ThreadPane";
import type { FeedMessage } from "./messageFormat";

const root: FeedMessage = {
  id: "root-1",
  user_id: "user-carol",
  content: "リリースノートの下書きを共有しました。レビューをお願いします。",
  created_at: "2026-01-02 09:05:00.000",
  parent_id: null,
  user_name: "carol",
  display_name: "Carol",
};

const replies: FeedMessage[] = [
  {
    id: "reply-1",
    user_id: "user-bob",
    content: "確認しました。2 点だけコメントしました。",
    created_at: "2026-01-02 09:06:00.000",
    parent_id: "root-1",
    user_name: "bob",
    display_name: "Bob",
  },
  {
    id: "reply-2",
    user_id: "user-alice",
    content: "ありがとうございます。修正して再共有します。",
    created_at: "2026-01-02 09:07:00.000",
    parent_id: "root-1",
    user_name: "alice",
    display_name: "Alice",
  },
  {
    id: "reply-3",
    user_id: "user-carol",
    content: "助かります。デプロイ後に確認します 🎉",
    created_at: "2026-01-02 09:08:00.000",
    parent_id: "root-1",
    user_name: "carol",
    display_name: "Carol",
  },
];

const meta = {
  title: "Features/Chat/ThreadPane",
  component: ThreadPane,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    isLoading: { control: "boolean" },
    isError: { control: "boolean" },
    isMember: { control: "boolean" },
  },
  args: {
    root,
    replies: [],
    isLoading: false,
    isError: false,
    currentUserId: "user-alice",
    reactions: new Map(),
    kitchenCatalog: null,
    canReact: true,
    onReaction: fn(),
    onPickReaction: fn(),
    editingId: null,
    editingText: "",
    onEditingTextChange: fn(),
    onStartEdit: fn(),
    onCancelEdit: fn(),
    onSaveEdit: fn(),
    onRequestDelete: fn(),
    onSendReply: async () => true,
    onClose: fn(),
  },
  decorators: [
    (Story) => (
      <div className="flex h-[640px] justify-end bg-muted/30">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ThreadPane>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Empty: Story = {};

export const WithReplies: Story = {
  args: {
    replies,
    reactions: new Map([["reply-1", [{ emoji: "👍", count: 2, ownId: null }]]]),
  },
};

export const Loading: Story = {
  args: {
    isLoading: true,
  },
};

export const Error: Story = {
  args: {
    isError: true,
  },
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    replies: replies.slice(0, 2),
  },
  decorators: [
    (Story) => (
      <div className="h-[640px] bg-muted/30">
        <Story />
      </div>
    ),
  ],
};
