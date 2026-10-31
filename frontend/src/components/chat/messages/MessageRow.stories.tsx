import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test";

import { StoryShell } from "../../../../.storybook/shell";
import type { AttachmentRow } from "../../../lib/electric/types";
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
  title: "Features/Chat/MessageRow",
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

const attachmentRows: AttachmentRow[] = [
  {
    id: "attachment-image-1",
    message_id: "message-1",
    uploader_id: "user-1",
    channel_id: "channel-general",
    file_name: "screenshot.png",
    file_size: "184320",
    content_type: "image/png",
    storage_key: "attachments/channel-general/attachment-image-1",
    created_at: "2026-01-02 09:00:00.000",
  },
  {
    id: "attachment-image-2",
    message_id: "message-1",
    uploader_id: "user-1",
    channel_id: "channel-general",
    file_name: "diagram.png",
    file_size: "98304",
    content_type: "image/png",
    storage_key: "attachments/channel-general/attachment-image-2",
    created_at: "2026-01-02 09:00:01.000",
  },
  {
    id: "attachment-file",
    message_id: "message-1",
    uploader_id: "user-1",
    channel_id: "channel-general",
    file_name: "release-notes.pdf",
    file_size: "524288",
    content_type: "application/pdf",
    storage_key: "attachments/channel-general/attachment-file",
    created_at: "2026-01-02 09:00:02.000",
  },
];

export const WithAttachments: Story = {
  args: {
    item: {
      ...item,
      message: { ...item.message, content: "スクリーンショットを共有します。" },
    },
    attachments: attachmentRows,
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

export const AppMessage: Story = {
  decorators: [
    (Story) => (
      <StoryShell>
        <Story />
      </StoryShell>
    ),
  ],
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("button", { name: "詳細を確認" })).toBeVisible();
  },
  parameters: { a11y: { test: "error" } },
  args: {
    item: {
      ...item,
      message: {
        ...item.message,
        is_bot: true,
        app_id: "echo",
        display_name: "Echo Bot",
        content: "Appからの通知",
        app_blocks: JSON.stringify([
          { type: "text", text: "デプロイ完了" },
          { type: "button", action_id: "details", label: "詳細を確認" },
        ]),
      },
    },
  },
};

export const CopyOriginalMenu: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    isOwn: false,
    canReact: false,
    item: {
      ...item,
      message: {
        ...item.message,
        content: "**整形前の本文**\n[リンク](https://example.com)\n:smile:",
      },
    },
  },
  play: async ({ canvasElement }) => {
    within(canvasElement).getByRole("button", { name: "その他の操作" }).focus();
    await userEvent.keyboard("{Enter}");
    await waitFor(() =>
      expect(screen.getByRole("menuitem", { name: "原文をコピー" })).toBeVisible(),
    );
  },
};

export const Edited: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    item: {
      ...item,
      message: {
        ...item.message,
        content: "編集後の本文 [リンク](https://example.com/guide)",
        updated_at: "2026-01-02 09:10:00.000",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const marker = canvas.getByRole("img", { name: "編集済み" });
    await expect(marker).toBeVisible();
    await userEvent.hover(marker);
    await expect(await screen.findByRole("tooltip")).toHaveTextContent("2026");
    await expect(canvas.getByRole("link", { name: "リンク" })).toHaveAttribute(
      "href",
      "https://example.com/guide",
    );
  },
};
