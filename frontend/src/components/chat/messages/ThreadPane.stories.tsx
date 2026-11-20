import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import { StoryShell } from "../../../../.storybook/shell";
import type { Decorator, Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, screen, userEvent, within } from "storybook/test";

import { MfmChannelsProvider } from "./MfmChannels";
import { storyChannels, storyUsers } from "../../../../.storybook/fixtures";
import { reactionUserLabel } from "../../../lib/emoji/reactions";
import { storyCommands, storyDeploymentCard } from "../../../../.storybook/app-fixtures";
import { MessageDecryptionsProvider } from "../../../lib/e2ee/messageDecryption";
import { ThreadPane } from "./ThreadPane";
import { ThreadPaneStatus } from "./ThreadPaneStatus";
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
    reactions: new Map([
      [
        "reply-1",
        [
          {
            emoji: "👍",
            count: 2,
            ownId: null,
            userIds: storyUsers.slice(0, 2).map((user) => user.id),
            userNames: Object.fromEntries(
              storyUsers.map((user) => [user.id, reactionUserLabel(user)]),
            ),
          },
        ],
      ],
    ]),
  },
};

export const Archived: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies, isArchived: true, currentUserId: null, canReact: false },
  play: playForImplementation(async ({ args }) => {
    // 狭幅ではダイアログが Portal に出るため canvasElement ではなく screen で探す。
    await expect(await screen.findByText("アーカイブ済み")).toBeVisible();
    await expect(screen.queryByRole("button", { name: "スレッドを再開" })).not.toBeInTheDocument();
    const input = screen.getByRole("textbox", { name: "返信を送信" });
    await userEvent.type(input, "このスレッドで続きを話します");
    await userEvent.click(screen.getByRole("button", { name: "送信" }));
    await expect(args.onSendReply).toHaveBeenCalledWith("このスレッドで続きを話します");
  }),
};

export const ArchivedReadOnly: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies, isArchived: true, readOnly: true, currentUserId: null, canReact: false },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByText("アーカイブ済み")).toBeVisible();
    await expect(canvas.queryByRole("textbox", { name: "返信を送信" })).not.toBeInTheDocument();
  }),
};

export const MobileArchived: Story = {
  ...Archived,
  globals: { viewport: { value: "mobile" } },
};

export const ArchivedNotJoined: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies, isArchived: true, isMember: false, currentUserId: null, canReact: false },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("textbox", { name: "返信を送信" })).toBeDisabled();
  }),
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

// 狭幅のオーバーレイはダイアログとして開く。Escape で閉じる要求が呼び出し元に届くことを確認する。
export const MobileDialog: Story = {
  globals: { viewport: { value: "mobile" } },
  parameters: { a11y: { test: "error" } },
  args: { replies },
  play: playForImplementation(async ({ args }) => {
    const dialog = await screen.findByRole("dialog", { name: "スレッド" });
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await userEvent.keyboard("{Escape}");
    await expect(args.onClose).toHaveBeenCalled();
  }),
};

export const Completions: Story = {
  parameters: { a11y: { test: "error" } },
  play: playForImplementation(async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    const input = await canvas.findByRole("textbox");
    await userEvent.type(input, "#gen");
    await userEvent.click(canvas.getByRole("option", { name: "#general" }));
    await expect(input).toHaveValue("#general ");
    await userEvent.clear(input);
    await userEvent.type(input, "/git");
    await userEvent.keyboard("{Enter}");
    await expect(input).toHaveValue("/github ");
    await userEvent.click(canvas.getByRole("button", { name: "送信" }));
    await expect(args.onSendReply).toHaveBeenCalledWith("/github", "github");
  }),
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
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      await canvas.findByRole("link", { name: storyDeploymentCard.title }),
    ).toBeVisible();
    await expect(
      canvas.queryByText("Plain fallback for card-capable clients"),
    ).not.toBeInTheDocument();
  }),
};

export const RootLoadError: Story = {
  parameters: { a11y: { test: "error" } },
  args: { onRetry: fn(), onClose: fn() },
  render: (args) => <ThreadPaneStatus isError onRetry={args.onRetry!} onClose={args.onClose} />,
  play: playForImplementation(async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("alert")).toBeVisible();
    await userEvent.click(canvas.getByRole("button", { name: "再試行" }));
    await expect(args.onRetry).toHaveBeenCalledTimes(1);
    await userEvent.click(canvas.getByRole("button", { name: "スレッドを閉じる" }));
    await expect(args.onClose).toHaveBeenCalledTimes(1);
  }),
};

export const NotParticipating: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies, membership: "none", participantCount: 2, onJoinThread: fn() },
  play: playForImplementation(async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "参加する" }));
    await expect(args.onJoinThread).toHaveBeenCalledOnce();
    await expect(canvas.queryByRole("button", { name: "スレッドを開く" })).not.toBeInTheDocument();
  }),
};
export const Participating: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies, membership: "joined", onLeaveThread: fn() },
  play: playForImplementation(async ({ canvasElement, args }) => {
    await userEvent.click(within(canvasElement).getByRole("button", { name: "退出する" }));
    await expect(args.onLeaveThread).toHaveBeenCalledOnce();
  }),
};
export const WithParticipants: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    replies,
    membership: "joined",
    onLeaveThread: fn(),
    participantCount: 53,
    participants: [
      { id: "u1", user_name: "alice", display_name: "Alice", avatar_url: null },
      { id: "u2", user_name: "bob", display_name: null, avatar_url: null },
    ],
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const toggle = canvas.getByRole("button", { name: "参加者 53人" });
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(toggle);
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    const list = within(canvas.getByRole("list", { name: "参加者 53人" }));
    await expect(list.getByText("Alice")).toBeVisible();
    await expect(list.getByText("@bob")).toBeVisible();
    await expect(list.getByText("ほか 51人")).toBeVisible();
  }),
};
export const ReopenAndJoin: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    replies,
    isArchived: true,
    membership: "none",
    participantCount: 2,
    onReopenAndJoinThread: fn(),
  },
  play: playForImplementation(async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await expect(canvas.queryByRole("textbox", { name: "返信を送信" })).not.toBeInTheDocument();
    // 入力欄が無いので「送信すると再Openして参加します」の注記も出さない。
    await expect(canvas.queryByText("送信すると再Openして参加します。")).not.toBeInTheDocument();
    await userEvent.click(canvas.getByRole("button", { name: "再Openして参加する" }));
    await expect(args.onReopenAndJoinThread).toHaveBeenCalledOnce();
  }),
};
export const MembershipLoadingBlocksComposer: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies, membershipLoading: true },
  play: playForImplementation(async ({ canvasElement }) => {
    await expect(
      await within(canvasElement).findByRole("textbox", { name: "返信を送信" }),
    ).toBeDisabled();
  }),
};
export const LeftWithNoParticipants: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies: [], membership: "left", participantCount: 0, onJoinThread: fn() },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("button", { name: "参加する" })).toBeEnabled();
    await expect(canvas.queryByRole("textbox", { name: "返信を送信" })).not.toBeInTheDocument();
  }),
};
export const NotJoinedEveryoneLeft: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    replies: [],
    membership: "none",
    participantCount: 0,
    threadTracked: true,
    onJoinThread: fn(),
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("button", { name: "参加する" })).toBeEnabled();
    await expect(canvas.queryByRole("textbox", { name: "返信を送信" })).not.toBeInTheDocument();
  }),
};
export const PostingForbidden: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    replies,
    membership: "joined",
    participantCount: 2,
    postingForbidden: true,
    composeDisabledReason: "このチャンネルは管理者のみ投稿できます。",
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // 閲覧はできるまま、入力欄だけが理由付きで止まる。
    await expect(await canvas.findByRole("textbox", { name: "返信を送信" })).toBeDisabled();
    await expect(canvas.getByText("このチャンネルは管理者のみ投稿できます。")).toBeInTheDocument();
  }),
};
export const PostingForbiddenNotJoined: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    replies,
    membership: "left",
    participantCount: 2,
    postingForbidden: true,
    composeDisabledReason: "このチャンネルは管理者のみ投稿できます。",
    onJoinThread: fn(),
  },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("このチャンネルは管理者のみ投稿できます。")).toBeVisible();
    // 参加は投稿権限と無関係にできる。返信を促す文は出さない。
    await expect(canvas.getByRole("button", { name: "参加する" })).toBeEnabled();
    await expect(
      canvas.queryByText("参加するとこのスレッドに返信できます。"),
    ).not.toBeInTheDocument();
  }),
};
export const FirstReplyWithoutJoining: Story = {
  parameters: { a11y: { test: "error" } },
  args: { replies: [], membership: "none", participantCount: 0, onJoinThread: fn() },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByRole("textbox", { name: "返信を送信" })).toBeEnabled();
    // 投稿せずに参加する導線も残す。
    await expect(canvas.getByRole("button", { name: "参加する" })).toBeEnabled();
  }),
};
export const LeftThread: Story = {
  ...NotParticipating,
  args: { ...NotParticipating.args, membership: "left" },
};
export const MembershipPending: Story = {
  ...NotParticipating,
  args: { ...NotParticipating.args, membershipPending: true },
  play: undefined,
};
export const MembershipFailed: Story = {
  ...NotParticipating,
  args: { ...NotParticipating.args, membershipError: true, onMembershipRetry: fn() },
  play: undefined,
};
export const ReopenArchived: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    replies,
    isArchived: true,
    membership: "joined",
    onReopenThread: fn(),
    onLeaveThread: fn(),
  },
  play: playForImplementation(async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "再Open" }));
    await expect(args.onReopenThread).toHaveBeenCalledOnce();
    await expect(canvas.getByText("送信すると再Openして参加します。")).toBeVisible();
  }),
};

export const LegacyNestedReply: Story = {
  parameters: { a11y: { test: "error" } },
  args: {
    replies: [
      {
        ...replies[0]!,
        display_data: { emojis: {}, attachments: [], reactions: [], reply_count: 2 },
      },
      replies[1]!,
    ],
    onOpenThread: fn(),
  },
  play: playForImplementation(async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /2.*返信/ }));
    await expect(args.onOpenThread).toHaveBeenCalledWith("reply-1");
  }),
};

// 暗号化チャンネルで、チャンネルのうち最初の暗号化したメッセージより後の平文の行（正しいサーバーは作れない）に、
// スレッドのペインでも警告を付ける（#367）。それより前の行（切り替える前のメッセージ）には付けない。
const encryptedSince =
  (since: string): Decorator =>
  (Story) => (
    <MessageDecryptionsProvider
      decryptions={new Map()}
      mentionUserNames={new Map()}
      encrypted
      encryptedSince={since}
    >
      <Story />
    </MessageDecryptionsProvider>
  );

export const EncryptedRepliesAfterEncryption: Story = {
  decorators: [encryptedSince("2026-01-02 09:05:30.000")],
  args: { replies },
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // 親（09:05）は切り替える前なので印を付けず、後の返信 3 件だけに警告を付ける。
    await expect(await canvas.findAllByText("暗号化されていません")).toHaveLength(3);
  }),
};

export const EncryptedRootAfterEncryption: Story = {
  decorators: [encryptedSince("2026-01-02 09:00:00.000")],
  play: playForImplementation(async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    // 親も、最初の暗号化したメッセージより後なら警告を付ける。
    await expect(await canvas.findAllByText("暗号化されていません")).toHaveLength(1);
  }),
};
