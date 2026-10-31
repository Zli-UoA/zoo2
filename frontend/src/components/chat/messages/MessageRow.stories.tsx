import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, waitFor, within } from "storybook/test";

import { StoryShell } from "../../../../.storybook/shell";
import { storyUsers } from "../../../../.storybook/fixtures";
import { reactionUserLabel } from "../../../lib/emoji/reactions";
import { storyAttachments } from "../../../../.storybook/attachment-fixtures";
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
const reactionUsers = Object.fromEntries(
  storyUsers.map((user) => [user.id, reactionUserLabel(user)]),
);
const reactionIds = storyUsers.slice(0, 3).map((user) => user.id);

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
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.hover(canvas.getByText(/こんにちは/));
    await expect(canvas.getByRole("button", { name: "編集" })).toBeInTheDocument();
  }),
};

export const WithReactions: Story = {
  args: {
    isOwn: true,
    replyCount: 4,
    onOpenThread: fn(),
    reactions: [
      {
        emoji: "👍",
        count: 3,
        ownId: "reaction-1",
        userIds: reactionIds,
        userNames: reactionUsers,
      },
      {
        emoji: "🎉",
        count: 1,
        ownId: null,
        userIds: reactionIds.slice(0, 1),
        userNames: reactionUsers,
      },
      {
        emoji: "kitchen:1f431_1f600",
        count: 2,
        ownId: null,
        userIds: reactionIds.slice(0, 2),
        userNames: reactionUsers,
      },
    ],
  },
};

export const ReactionUsers: Story = {
  globals: { viewport: { value: "mobile" } },
  parameters: { a11y: { test: "error" } },
  args: {
    reactions: [
      {
        emoji: "👍",
        count: 2,
        ownId: null,
        userIds: ["long", "user-bob"],
        userNames: { ...reactionUsers, long: `${"A".repeat(80)} (@longname)` },
      },
    ],
  },
  play: playForImplementation(async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByRole("button", { name: "👍 にリアクションしたユーザーを表示" }),
    );
    const dialog = await screen.findByRole("dialog");
    await waitFor(() => expect(within(dialog).getByText(/@longname/)).toBeVisible());
    await waitFor(() => expect(within(dialog).getByText(/@bob/)).toBeVisible());
  }),
};

export const ReactionUsersHover: Story = {
  parameters: { a11y: { test: "error" } },
  args: WithReactions.args,
  play: playForImplementation(async ({ canvasElement }) => {
    const chip = within(canvasElement).getByRole("button", { name: /👍 3 件/ });
    await userEvent.hover(chip);
    const tooltip = await screen.findByRole("tooltip");
    await expect(tooltip).toHaveTextContent("@alice");
  }),
};

export const ReplyCount: Story = {
  args: {
    replyCount: 3,
    onOpenThread: fn(),
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("button", { name: /3 件の返信/ })).toBeVisible();
  }),
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

export const WithAttachments: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    item: {
      ...item,
      message: { ...item.message, content: "スクリーンショットを共有します。" },
    },
    attachments: storyAttachments,
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(
      await canvas.findByRole("button", { name: "screenshot.pngをプレビュー" }),
    );
    const dialog = await screen.findByRole("dialog");
    await waitFor(async () => {
      const preview = within(dialog).getByRole("img", { name: "screenshot.png" });
      await expect(preview).toBeVisible();
      await expect((preview as HTMLImageElement).naturalWidth).toBeGreaterThan(0);
    });
    await userEvent.click(within(dialog).getByRole("button", { name: "閉じる" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  }),
};

export const Mobile: Story = {
  globals: { viewport: { value: "mobile" } },
  args: {
    isOwn: true,
    replyCount: 2,
    onOpenThread: fn(),
    reactions: [
      {
        emoji: "👍",
        count: 2,
        ownId: "reaction-1",
        userIds: reactionIds.slice(0, 2),
        userNames: reactionUsers,
      },
    ],
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
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(within(canvasElement).getByRole("button", { name: "詳細を確認" })).toBeVisible();
  }),
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
  play: playForImplementation(async ({ canvasElement }) => {
    within(canvasElement).getByRole("button", { name: "その他の操作" }).focus();
    await userEvent.keyboard("{Enter}");
    await waitFor(() =>
      expect(screen.getByRole("menuitem", { name: "原文をコピー" })).toBeVisible(),
    );
  }),
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
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const marker = canvas.getByRole("img", { name: "編集済み" });
    await expect(marker).toBeVisible();
    await userEvent.hover(marker);
    await expect(await screen.findByRole("tooltip")).toHaveTextContent("2026");
    await expect(canvas.getByRole("link", { name: "リンク" })).toHaveAttribute(
      "href",
      "https://example.com/guide",
    );
  }),
};

export const Grouped: Story = { args: { item: { ...item, isGroupStart: false } } };
