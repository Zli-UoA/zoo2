import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

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

const longContent = Array.from(
  { length: 8 },
  (_, index) =>
    `${index + 1}. リリース手順の確認項目です。担当者と期限を確認してから次へ進んでください。`,
).join("\n");

const meta = {
  title: "Chat/MessageRow",
  component: MessageRow,
  tags: ["autodocs"],
  parameters: { a11y: { test: "todo" } },
  argTypes: {
    isOwn: { control: "boolean" },
    isEditing: { control: "boolean" },
    replyCount: { control: { type: "number", min: 0 } },
    canReact: { control: "boolean" },
  },
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

export const OwnMessage: Story = {
  args: {
    isOwn: true,
  },
};

export const HoverActions: Story = {
  args: {
    isOwn: true,
    onOpenThread: fn(),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.hover(canvas.getByText(/こんにちは/));
    await expect(canvas.getByRole("button", { name: "編集" })).toBeInTheDocument();
  },
};

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

export const ReplyCount: Story = {
  args: {
    replyCount: 3,
    onOpenThread: fn(),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("button", { name: /3 件の返信/ })).toBeVisible();
  },
};

export const Editing: Story = {
  args: {
    isOwn: true,
    isEditing: true,
    editingText: "こんにちは。今日のリリースは 18:00 です。",
  },
};

export const LongMessage: Story = {
  args: {
    item: {
      ...item,
      message: { ...item.message, content: longContent },
    },
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

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    isOwn: true,
    replyCount: 2,
    onOpenThread: fn(),
    reactions: [{ emoji: "👍", count: 2, ownId: "reaction-1" }],
  },
};
