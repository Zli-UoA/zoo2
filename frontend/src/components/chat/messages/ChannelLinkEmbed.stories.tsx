import { storySurfaces } from "../../../../.storybook/story-surfaces";
import { playForImplementation } from "../../../../.storybook/play-for-implementation";
import type { Decorator, Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, waitFor, within } from "storybook/test";
import type { ChannelRow, MessageRow as MessageData, UserRow } from "../../../lib/electric/types";
import { MessageBody, MessageRow } from "./MessageRow";
import { MfmChannelsProvider } from "./MfmChannels";

/**
 * メッセージ・チャンネルのリンクの、短い表示と埋め込みカード。本文の組み立ては実アプリと同じ `MessageBody` で、
 * 行き先は Story のコレクション（`parameters.collections`）から取る。見られるチャンネルの一覧は、
 * アプリのシェルが配る一覧と同じ形で `MfmChannelsProvider` に渡す。
 */

const ts = (minute: number) => `2026-11-23 07:${String(minute).padStart(2, "0")}:00.000`;

function channel(overrides: Partial<ChannelRow> & Pick<ChannelRow, "id" | "channel_name">) {
  return {
    description: null,
    created_user_id: "u-alice",
    is_private: false,
    is_dm: false,
    created_at: ts(0),
    updated_at: null,
    archive_at: null,
    deleted_at: null,
    imported_from_slack: false,
    slack_import_kind: null,
    slack_imported_at: null,
    ...overrides,
  } satisfies ChannelRow;
}

function message(overrides: Partial<MessageData> & Pick<MessageData, "id" | "channel_id">) {
  return {
    user_id: "u-ames",
    content: "テスト",
    created_at: ts(30),
    updated_at: null,
    deleted_at: null,
    parent_id: null,
    ...overrides,
  } satisfies MessageData;
}

const users: UserRow[] = [
  { id: "u-alice", user_name: "alice", display_name: "Alice", created_at: ts(0), updated_at: null },
  {
    id: "u-ames",
    user_name: "amesyu",
    display_name: "アメシュ",
    created_at: ts(0),
    updated_at: null,
  },
];

const general = channel({
  id: "c-general",
  channel_name: "z_-amesyu",
  description: "場所の共有と雑談。リンクの表示を確認するための説明文です。",
});
const projects = channel({
  id: "c-projects",
  channel_name: "プロジェクト・リリース計画とレビュー運用のための非常に長いチャンネル名",
  is_private: true,
});
const sealed = channel({
  id: "c-sealed",
  channel_name: "sealed",
  is_private: true,
  encrypted: true,
});
const dm = channel({
  id: "c-dm",
  channel_name: "dm:u-alice:u-ames",
  is_private: true,
  is_dm: true,
});
/** 自分が参加していない非公開チャンネルは、一覧にも来ない（ここにだけ置いて、一覧から外す）。 */
const hidden = channel({ id: "c-hidden", channel_name: "秘密の計画", is_private: true });

/** サーバーが `encrypted = false` を配っているが、暗号文のある行を含むチャンネル（格下げ）。 */
const downgraded = channel({ id: "c-downgraded", channel_name: "downgraded" });

const visibleChannels = [general, projects, sealed, dm, downgraded];

const storyMessages: MessageData[] = [
  message({ id: "m-1", channel_id: general.id, content: "テスト" }),
  message({
    id: "m-reply",
    channel_id: general.id,
    parent_id: "m-1",
    content: "スレッドへの返信です。**太字**や[リンク](https://example.com)は文字だけにします。",
    created_at: ts(31),
  }),
  message({
    id: "m-long",
    channel_id: projects.id,
    content:
      "長い本文は 3 行で切ります。リリースの手順、確認する項目、担当者の割り当て、レビューの期限、障害時の連絡先をまとめました。".repeat(
        4,
      ),
  }),
  message({ id: "m-sealed", channel_id: sealed.id, content: "" }),
  message({ id: "m-dm", channel_id: dm.id, content: "DM の内容です。" }),
  message({
    id: "m-files",
    channel_id: general.id,
    content: "",
    display_data: {
      emojis: {},
      attachments: [{ id: "a1" }, { id: "a2" }] as never,
      reactions: [],
      reply_count: 0,
    },
  }),
  message({ id: "m-deleted", channel_id: general.id, content: "消えた", deleted_at: ts(40) }),
  message({
    id: "m-card",
    channel_id: general.id,
    content: "カードの代わりの本文",
    card: JSON.stringify({
      title: "デプロイ完了",
      description: "",
      url: "https://example.com",
      source: "CI",
      context: "",
      author: "",
      badge: "",
    }),
  }),
  // 格下げされたチャンネルの、暗号文の行と、その隣の平文の行（サーバーが差し込んだ行かもしれない）。
  message({
    id: "m-cipher",
    channel_id: downgraded.id,
    content: "",
    ciphertext: {
      algorithm: "m.megolm.v1.aes-sha2",
      sender_key: "key",
      device_id: "device",
      session_id: "session",
      ciphertext: "AwgA",
    },
  }),
  message({
    id: "m-plain-sibling",
    channel_id: downgraded.id,
    content: "暗号文の隣の平文",
    created_at: ts(31),
  }),
];

const withDirectory =
  (ready = true): Decorator =>
  (Story) => (
    <MfmChannelsProvider channels={visibleChannels} ready={ready}>
      <Story />
    </MfmChannelsProvider>
  );

const meta = {
  title: "Features/Chat/ChannelLinkEmbed",
  component: MessageBody,
  parameters: {
    a11y: { test: "error" },
    collections: {
      currentUserId: "u-alice",
      users,
      channels: [...visibleChannels, hidden],
      messages: storyMessages,
    },
  },
  decorators: [
    withDirectory(),
    (Story) => (
      <div className={storySurfaces["Features/Chat/ChannelLinkEmbed"]}>
        <Story />
      </div>
    ),
  ],
  args: { content: "" },
  render: (args) => {
    // `content` は、Story の `args.content`（`{origin}` 付きの URL）を、描くときに組み立てる。
    return (
      <MessageBody
        {...args}
        content={args.content.replaceAll("{origin}", window.location.origin)}
      />
    );
  },
} satisfies Meta<typeof MessageBody>;
export default meta;
type Story = StoryObj<typeof meta>;

const url = (path: string) => `{origin}/channels/${path}`;

/**
 * 描画が落ち着くのを待ちながら検証する。ストーリーは、開始直後（`globals` の適用など）に作り直されることが
 * あり、最初に見つけた要素が古くなる。要素を持ち回らず、毎回探し直す。
 */
const settled = (
  check: (canvas: ReturnType<typeof within>, root: HTMLElement) => Promise<void> | void,
) =>
  playForImplementation(async ({ canvasElement }) => {
    await waitFor(async () => check(within(canvasElement), canvasElement));
  });

/** 参考にした Slack の表示。短くしたリンクと、投稿者・チャンネル・本文の埋め込み。 */
export const Message: Story = {
  args: { content: `こうだと？\n${url("c-general?message=m-1")}` },
  play: settled(async (canvas, root) => {
    await expect(canvas.getByRole("link", { name: "アメシュさん: #z_-amesyu" })).toBeVisible();
    const embed = canvas.getByRole("link", { name: /アメシュ.*テスト/ });
    await expect(embed).toHaveAttribute("href", "/channels/c-general?message=m-1");
    await expect(within(embed).getByText("テスト")).toBeVisible();
    // 元の URL は表示しない（表示では `https://` を省くので、パスで確かめる）。
    await expect(root.textContent).not.toContain("/channels/");
  }),
};

export const ThreadReply: Story = {
  args: { content: url("c-general?thread=m-1&message=m-reply") },
  play: settled(async (canvas) => {
    const embed = canvas.getByRole("link", { name: /スレッドへの返信/ });
    await expect(within(embed).getByText("#z_-amesyu 内のスレッド")).toBeVisible();
    await expect(embed).toHaveAttribute("href", "/channels/c-general?thread=m-1&message=m-reply");
    // 装飾とリンクは文字だけにして、リンクの中にリンクを作らない。
    await expect(embed.querySelector("a, strong")).toBeNull();
  }),
};

/** 暗号化チャンネルのメッセージは、本文を出さず、暗号化されていることを書く。 */
export const Encrypted: Story = {
  args: { content: url("c-sealed?message=m-sealed") },
  play: settled(async (canvas) => {
    const embed = canvas.getByRole("link", { name: /このメッセージは暗号化されています/ });
    await expect(within(embed).getByText("#sealed")).toBeVisible();
    await expect(embed).toHaveAttribute("href", "/channels/c-sealed?message=m-sealed");
  }),
};

/**
 * 暗号文のある行を含むチャンネルは、`encrypted = false` が配られていても、暗号化チャンネルとして扱う
 * （格下げ対策）。暗号文の行も、その隣の平文の行も、本文を出さず暗号化と書く。
 */
export const CiphertextInPlainChannel: Story = {
  args: {
    content: `${url("c-downgraded?message=m-cipher")}\n${url("c-downgraded?message=m-plain-sibling")}`,
  },
  play: settled(async (canvas, root) => {
    await expect(
      canvas.getAllByRole("link", { name: /このメッセージは暗号化されています/ }),
    ).toHaveLength(2);
    await expect(root.textContent).not.toContain("暗号文の隣の平文");
  }),
};

/** 連携カード・部品の投稿は、チャンネルでは `content` の代わりにそれらを描く。埋め込みも本文を出さない。 */
export const AppMessage: Story = {
  args: { content: url("c-general?message=m-card") },
  play: settled(async (canvas, root) => {
    await expect(canvas.getByRole("link", { name: /Appのメッセージ/ })).toBeVisible();
    await expect(root.textContent).not.toContain("カードの代わりの本文");
  }),
};

/** 参加していない非公開チャンネルは、存在しないものとして扱う。チャンネル名も本文も出さない。 */
export const NotJoined: Story = {
  args: { content: url("c-hidden?message=m-hidden") },
  play: settled(async (canvas, root) => {
    await expect(canvas.getByText("メッセージを表示できません")).toBeVisible();
    await expect(root.textContent).not.toContain("秘密の計画");
    await expect(root.textContent).not.toContain("見えてはいけない");
    // 埋め込みは押せない。本文中の URL だけがリンクとして残る。
    await expect(canvas.getAllByRole("link")).toHaveLength(1);
  }),
};

export const MessageDeleted: Story = {
  args: { content: url("c-general?message=m-deleted") },
  play: settled(async (canvas, root) => {
    await expect(canvas.getByText("メッセージを表示できません")).toBeVisible();
    await expect(root.textContent).not.toContain("消えた");
  }),
};

export const MessageNotFound: Story = {
  args: { content: url("c-general?message=m-nothing") },
  play: settled(async (canvas) => {
    await expect(canvas.getByText("メッセージを表示できません")).toBeVisible();
  }),
};

export const Channel: Story = {
  args: { content: `チャンネルはこちら ${url("c-general")}` },
  play: settled(async (canvas) => {
    await expect(canvas.getByRole("link", { name: "#z_-amesyu" })).toBeVisible();
    await expect(canvas.getByRole("link", { name: /場所の共有と雑談/ })).toHaveAttribute(
      "href",
      "/channels/c-general",
    );
  }),
};

export const PrivateAndEncryptedChannels: Story = {
  args: { content: `${url("c-projects")}\n${url("c-sealed")}` },
  play: settled(async (canvas) => {
    await expect(canvas.getByText("非公開チャンネル")).toBeVisible();
    await expect(canvas.getByText("暗号化チャンネル")).toBeVisible();
  }),
};

export const ChannelNotJoined: Story = {
  args: { content: url("c-hidden") },
  play: settled(async (canvas) => {
    await expect(canvas.getByText("チャンネルを表示できません")).toBeVisible();
  }),
};

export const DirectMessage: Story = {
  args: { content: url("c-dm?message=m-dm") },
  play: settled(async (canvas, root) => {
    await expect(
      canvas.getByRole("link", { name: "アメシュさん: ダイレクトメッセージ" }),
    ).toBeVisible();
    // 内部の名前（dm:…）は出さない。
    await expect(root.textContent).not.toContain("dm:u-alice");
  }),
};

export const AttachmentsOnly: Story = {
  args: { content: url("c-general?message=m-files") },
  play: settled(async (canvas) => {
    await expect(canvas.getByText("添付ファイル 2 件")).toBeVisible();
  }),
};

/** 長い本文は 3 行で切り、長いチャンネル名は 1 行に収める。カードは幅を超えない。 */
export const LongText: Story = {
  args: { content: url("c-projects?message=m-long") },
  play: settled(async (canvas) => {
    const embed = canvas.getByRole("link", { name: /長い本文は/ });
    await expect(embed.getBoundingClientRect().width).toBeLessThanOrEqual(448);
    await expect(embed.scrollWidth).toBeLessThanOrEqual(embed.clientWidth);
  }),
};

/**
 * 参考にした Slack の表示と同じ構図。投稿者と時刻のある行の本文に、短くしたリンクを書き、その下に
 * 埋め込みを出す。返信（スレッド）へのリンクで、埋め込みに「内のスレッド」が付く。
 */
export const InMessageRow: Story = {
  render: () => (
    <MessageRow
      item={{
        message: {
          id: "row-1",
          channel_id: general.id,
          user_id: "u-ames",
          content: `こうだと？\n${window.location.origin}/channels/c-general?thread=m-1&message=m-reply`,
          created_at: ts(30),
          parent_id: null,
          user_name: "amesyu",
          display_name: "アメシュ",
        },
        isGroupStart: true,
        isGroupEnd: true,
        showDateSeparator: false,
      }}
      isOwn={false}
      isEditing={false}
      editingText=""
      onEditingTextChange={fn()}
      onStartEdit={fn()}
      onCancelEdit={fn()}
      onSaveEdit={fn()}
      onRequestDelete={fn()}
      reactions={[]}
      kitchenCatalog={null}
      canReact
      onReaction={fn()}
      onPickReaction={fn()}
    />
  ),
  play: settled(async (canvas) => {
    await expect(canvas.getByRole("link", { name: "アメシュさん: #z_-amesyu" })).toBeVisible();
    const embed = canvas.getByRole("link", { name: /スレッドへの返信/ });
    await expect(within(embed).getByText("#z_-amesyu 内のスレッド")).toBeVisible();
  }),
};

export const Mobile: Story = {
  ...Message,
  globals: { viewport: { value: "mobile" } },
};

/** 一覧にないチャンネルは、一覧の同期が終わるまでは、「表示できません」と決めず、枠だけを置く（一覧にあるチャンネルは、同期中でも表示する）。 */
export const LoadingDirectory: Story = {
  args: { content: url("c-hidden?message=m-hidden") },
  decorators: [withDirectory(false)],
  play: settled(async (canvas) => {
    // 枠は、読み上げに「読み込み中」と伝える。「表示できません」とは決めない。
    await expect(canvas.getByRole("status")).toHaveTextContent("リンク先を読み込み中");
    await expect(canvas.queryByText("メッセージを表示できません")).toBeNull();
  }),
};

/** 暗号化チャンネルの投稿では、外部の URL のカードを押したときだけ取得する。アプリ内のリンクはすぐ埋め込む。 */
export const InEncryptedPost: Story = {
  args: {
    content: `${url("c-general?message=m-1")}\nhttps://example.com/docs`,
    previews: "manual",
  },
  play: settled(async (canvas) => {
    await expect(canvas.getByRole("link", { name: /アメシュ.*テスト/ })).toBeVisible();
    await expect(
      canvas.getByRole("button", { name: "リンクのプレビューを読み込む" }),
    ).toBeVisible();
  }),
};
