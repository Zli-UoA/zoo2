import type { Meta, StoryObj } from "@storybook/react-vite";
import { fn } from "storybook/test";

import { MessageRow } from "./MessageRow";
import type { MessageItem } from "./messageGrouping";

const item: MessageItem = {
  message: {
    id: "message-1",
    user_id: "user-1",
    content:
      "こんにちは。今日のリリースは 18:00 です。詳細は https://example.com/docs を参照してください。",
    created_at: "2026-01-02 09:00:00.000",
    parent_id: null,
    user_name: "alice",
    display_name: "Alice",
  },
  isGroupStart: true,
  isGroupEnd: true,
  showDateSeparator: false,
};

const meta = {
  title: "Chat/MessageRow",
  component: MessageRow,
  args: {
    item,
    isOwn: false,
    isEditing: false,
    editingText: "",
    onEditingTextChange: fn(),
    onStartEdit: fn(),
    onCancelEdit: fn(),
    onSaveEdit: fn(),
    onRequestDelete: fn(),
    reactions: [],
    kitchenCatalog: null,
    canReact: true,
    onReaction: fn(),
    onPickReaction: fn(),
    replyCount: 0,
  },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-3xl bg-background py-8">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MessageRow>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const WithReactions: Story = {
  args: {
    isOwn: true,
    replyCount: 4,
    onOpenThread: fn(),
    reactions: [
      { emoji: "👍", count: 3, ownId: "reaction-1" },
      { emoji: "🎉", count: 1, ownId: null },
      { emoji: "kitchen:1f431_1f600", count: 2, ownId: null },
    ],
  },
};

export const Card: Story = {
  args: {
    item: {
      ...item,
      message: {
        ...item.message,
        content: "デプロイが完了しました",
        card: JSON.stringify({
          title: "Deploy complete",
          description: "production へのデプロイが完了しました。",
          source: "Deployment service",
          badge: "success",
        }),
      },
    },
  },
};
