import { StoryShell } from "../../../../.storybook/shell";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";

import { MfmChannelsProvider } from "./MfmChannels";
import { storyChannels } from "../../../../.storybook/fixtures";
import { storyCommands, storyDeploymentCard } from "../../../../.storybook/app-fixtures";
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
    commands: storyCommands,
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
    onSendReply: fn(async () => true),
    onClose: fn(),
  },
  decorators: [
    (Story) => (
      <div className="flex h-[640px] justify-end bg-muted/30">
        <MfmChannelsProvider channels={storyChannels}>
          <Story />
        </MfmChannelsProvider>
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
        <MfmChannelsProvider channels={storyChannels}>
          <Story />
        </MfmChannelsProvider>
      </div>
    ),
  ],
};

export const Completions: Story = {
  parameters: { a11y: { test: "error" } },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    const input = canvas.getByRole("textbox");
    await userEvent.type(input, "#gen");
    await userEvent.click(canvas.getByRole("option", { name: "#general" }));
    await expect(input).toHaveValue("#general ");
    await userEvent.clear(input);
    await userEvent.type(input, "/git");
    await userEvent.keyboard("{Enter}");
    await expect(input).toHaveValue("/github ");
    await userEvent.click(canvas.getByRole("button", { name: "送信" }));
    await expect(args.onSendReply).toHaveBeenCalledWith("/github", "github");
  },
};

export const AppCard: Story = {
  parameters: { a11y: { test: "error" } },
  decorators: [
    (Story) => (
      <StoryShell>
        <Story />
      </StoryShell>
    ),
  ],
  args: {
    root: {
      ...root,
      content: "Plain fallback for card-capable clients",
      app_origin_id: "github",
      app_blocks: JSON.stringify([{ type: "card", card: storyDeploymentCard }]),
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("link", { name: storyDeploymentCard.title })).toBeVisible();
    await expect(
      canvas.queryByText("Plain fallback for card-capable clients"),
    ).not.toBeInTheDocument();
  },
};
